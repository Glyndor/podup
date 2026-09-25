//! Per-service build: build one `build:` block end-to-end.
//!
//! The dispatching loop in `mod.rs` calls into [`Engine::build_service`],
//! which is the URL/stream glue for one image: pick a body plan,
//! assemble the build query, drive the chunked response, paint the
//! board row. Each step is small enough that the whole thing lives
//! here. The body-plan decision lives in [`super::body_plan`]; the
//! `apply_extra_tags` follow-up lives in [`super::extra_tags`].

use std::io::IsTerminal;

use bytes::Bytes;
use futures_util::StreamExt;
use tracing::warn;

use crate::compose::types::{BuildConfig, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::BuildOutput;
use crate::libpod::urlencoded;
use crate::libpod::validate::pre_validate_build;
use crate::libpod::API_PREFIX;
use crate::size;

use super::body_plan::plan_build;
use super::steps::{parse_image_id_line, BuildStreamProgress};
use super::stream::context_body;
use super::tags::looks_like_secret;
use super::{context::map_additional_context, Engine};
use super::{BodyPlan, BuildOptions};

impl Engine {
	pub(in crate::engine) async fn build_service(
		&self,
		service_name: &str,
		service: &Service,
		file: &crate::compose::types::ComposeFile,
		opts: &BuildOptions,
	) -> Result<()> {
		let build = match &service.build {
			Some(b) => b,
			None => return Ok(()),
		};

		let context_str = build.context().to_string();
		// `tag` is the un-normalised form the user (or `primary_build_tag`'s
		// `<project>-<service>:latest` default) supplied. It is what every
		// print path carries - the board row `up` and `build` seed, the
		// `Building`/`Built` verbs, the `STEP n/m:` line prefix, the
		// `fail_build` error message, and the `apply_extra_tags` comparison.
		// Only the wire query (`t=` and `/images/{}/tag`) carries the
		// docker.io canonical form, computed just before the query string is
		// assembled below.
		let tag = super::primary_build_tag(
			&self.project,
			service_name,
			service.image.as_deref(),
			build.tags(),
		);

		let plan = plan_build(self, service_name, service, file, build, &context_str)?;
		let body_plan = plan.body;
		let dockerfile_name = plan.dockerfile;
		let secret_specs = plan.secrets;

		let arg_map = build.args().to_map();
		let mut build_args: std::collections::HashMap<String, String> =
			std::collections::HashMap::new();
		for (k, v) in arg_map {
			let value = v.unwrap_or_else(|| std::env::var(&k).unwrap_or_default());
			build_args.insert(k, value);
		}
		// CLI `--build-arg KEY=VAL` overrides the compose `build.args`. A bare
		// `KEY` (no `=`) takes its value from the process environment, matching
		// docker compose.
		for entry in &opts.build_args {
			let (k, v) = match entry.split_once('=') {
				Some((k, v)) => (k.to_string(), v.to_string()),
				None => (entry.clone(), std::env::var(entry).unwrap_or_default()),
			};
			// A bare `=value` (empty key) is a user typo Podman would silently
			// ignore; reject it with a clear diagnostic instead.
			if k.is_empty() {
				return Err(ComposeError::Build(format!(
					"invalid --build-arg '{entry}': empty argument name"
				)));
			}
			build_args.insert(k, v);
		}

		// A secret passed as a build arg is recorded in the image history and, if
		// promoted via `ENV`, the image config, so it can leak. Warn and point at
		// `build.secrets` (BuildKit `--mount=type=secret`), which does not persist.
		let mut secretish: Vec<&str> = build_args
			.keys()
			.filter(|k| looks_like_secret(k))
			.map(String::as_str)
			.collect();
		if !secretish.is_empty() {
			secretish.sort_unstable();
			warn!(
				"build-arg(s) [{}] look like secrets; build args are stored in the image history \
				 and can leak. Use build.secrets for sensitive values.",
				secretish.join(", ")
			);
		}

		let mut labels: std::collections::BTreeMap<String, String> =
			std::collections::BTreeMap::new();
		if let BuildConfig::Config { labels: l, .. } = build {
			labels.extend(l.to_map());
		}
		// Stamp podup's ownership labels AFTER the user's `build.labels` so a
		// user label named `podup.project` cannot displace this project's
		// value. Without this, `build.labels: {podup.project: other}` would
		// make `podman image prune --filter label=podup.project=<self>`
		// miss every image this build produced and reach for `other`'s
		// instead. A `BTreeMap` (rather than the `HashMap` this used to be)
		// keeps the label order deterministic across builds, so a second
		// `podup build` of the same Containerfile hits the buildkit layer
		// cache instead of producing a different `LABEL` step every time.
		labels.insert("podup.project".to_string(), self.project.clone());
		labels.insert("podup.service".to_string(), service_name.to_string());

		// Pre-validate the keys libpod's buildkit-fronted parser rejects
		// (anything outside `[A-Za-z0-9_.-]`), so a bad `build.args` or
		// `build.labels` key surfaces as a `PodmanError::Field` naming the
		// compose-side field rather than libpod's `400` body. Runs before
		// the build URL is assembled so a bad key fails before any POST to
		// the daemon (#1357).
		pre_validate_build(&build_args, &labels)?;

		let network = if let BuildConfig::Config {
			network: Some(n), ..
		} = build
		{
			Some(n.clone())
		} else {
			None
		};
		let platform = match build {
			BuildConfig::Config { platforms, .. } => platforms.first().inspect(|first| {
				let rest_count = platforms.len() - 1;
				if rest_count > 0 {
					warn!("build.platforms: libpod builds one platform per request; building {first}, ignoring {rest_count} other(s)");
				}
			}).cloned(),
			_ => None,
		};
		// Distinguish "absent" from "present but unparseable": a malformed
		// `shm_size` must be rejected, not silently dropped to the default.
		let shmsize = match build.shm_size() {
			Some(raw) => Some(size::parse_memory(raw).ok_or_else(|| {
				ComposeError::Build(format!("invalid build.shm_size value '{raw}'"))
			})? as i32),
			None => None,
		};
		let extrahosts_str = build.extra_hosts().join(",");
		let extrahosts = if extrahosts_str.is_empty() {
			None
		} else {
			Some(extrahosts_str)
		};
		let cachefrom = if build.cache_from().is_empty() {
			None
		} else {
			Some(super::super::to_query_json(
				"build.cache_from",
				&build.cache_from(),
			)?)
		};
		let buildargs_json = if build_args.is_empty() {
			None
		} else {
			Some(super::super::to_query_json("build.args", &build_args)?)
		};
		let labels_json = if labels.is_empty() {
			None
		} else {
			Some(super::super::to_query_json("build.labels", &labels)?)
		};
		let build_ulimits = super::render_build_ulimits(build);
		let ulimits_json = if build_ulimits.is_empty() {
			None
		} else {
			Some(super::super::to_query_json(
				"build.ulimits",
				&build_ulimits,
			)?)
		};

		// `rm=true` removes intermediate containers only after a successful
		// build; `forcerm=true` removes them after a failure too. Podman's
		// `podman build` (and `podman --remote build`) default `--force-rm`
		// to true, and podup shipped without it: measured on 2026-09-24
		// against Podman 5.7.0, posting the same failing build to
		// `/v5.0.0/libpod/build` leaked one buildah working container on
		// every run without `forcerm` (2 of 2) and on none of the runs
		// with it (0 of 2).
		//
		// `outputformat=application/vnd.docker.distribution.manifest.v2+json`
		// forces the docker-distribution manifest format. Measured on
		// 2026-09-24 against Podman 5.7.0 by building the same
		// Containerfile twice through `/v5.0.0/libpod/build`:
		// `layers=true` alone prints zero `Using cache` lines on the
		// second build (the OCI format the endpoint defaults to does not
		// reuse the layer cache); the same query with
		// `outputformat=application/vnd.docker.distribution.manifest.v2+json`
		// appended prints two. The Docker format also keeps
		// `HEALTHCHECK` in the image config (the OCI format drops it),
		// which `podup`'s `healthcheck:` field inherits when the user
		// does not set one explicitly, so the same query preserves the
		// image shape podup has always produced.
		//
		// `t=` carries the docker.io canonical form (the compat build
		// handler applied `NormalizeToDockerHub`; the libpod path skips
		// it via `IsLibpodRequest`, so `podup build` would otherwise
		// land unqualified names as `localhost/<project>-<service>:latest`
		// instead of `docker.io/library/<project>-<service>:latest`).
		// The normalisation runs against a separate `wire_tag` rather
		// than mutating `tag`: every print path (`Building`/`Built`,
		// the board row `up` seeded, the `STEP n/m:` line prefix, the
		// `apply_extra_tags` comparison) keeps the un-normalised form,
		// which is what `podup ps`/`podup images` show and what the
		// user used to see (#1914).
		let wire_tag = self.normalize_image_reference(&tag).await?;
		let mut qs = format!(
			"t={}&rm=true&forcerm=true&layers=true&nocache={}&outputformat={}",
			urlencoded(&wire_tag),
			build.no_cache() || opts.no_cache,
			urlencoded("application/vnd.docker.distribution.manifest.v2+json"),
		);
		qs.push_str(&format!("&dockerfile={}", urlencoded(&dockerfile_name)));
		if build.pull() || opts.pull {
			qs.push_str("&pull=true");
		}
		if let Some(p) = &platform {
			qs.push_str(&format!("&platform={}", urlencoded(p)));
		}
		if let Some(n) = &network {
			qs.push_str(&format!("&networkmode={}", urlencoded(n)));
		}
		if let Some(s) = shmsize {
			qs.push_str(&format!("&shmsize={s}"));
		}
		if let Some(h) = &extrahosts {
			qs.push_str(&format!("&extrahosts={}", urlencoded(h)));
		}
		if let Some(c) = &cachefrom {
			qs.push_str(&format!("&cachefrom={}", urlencoded(c)));
		}
		if let Some(a) = &buildargs_json {
			qs.push_str(&format!("&buildargs={}", urlencoded(a)));
		}
		if let Some(l) = &labels_json {
			qs.push_str(&format!("&labels={}", urlencoded(l)));
		}
		// `layerLabel` is a repeated `key=value` query parameter that labels
		// every image the build produced, including the intermediate stage
		// images that `labels=` leaves unlabelled. Only podup's two keys go
		// here; the user's `build.labels` keep going through `labels=` so
		// their policy (label, not layer-label) is preserved. A multi-stage
		// build otherwise leaves every intermediate image unclaimed on
		// disk, untagged, and indistinguishable from another project's.
		qs.push_str(&format!(
			"&layerLabel={}&layerLabel={}",
			urlencoded(&format!("podup.project={}", self.project)),
			urlencoded(&format!("podup.service={}", service_name)),
		));
		if let Some(u) = &ulimits_json {
			qs.push_str(&format!("&ulimits={}", urlencoded(u)));
		}
		if let Some(target) = build.target() {
			qs.push_str(&format!("&target={}", urlencoded(target)));
		}
		if !secret_specs.is_empty() {
			let json = super::super::to_query_json("build.secrets", &secret_specs)?;
			qs.push_str(&format!("&secrets={}", urlencoded(&json)));
		}
		if !build.cache_to().is_empty() {
			let json = super::super::to_query_json("build.cache_to", &build.cache_to())?;
			qs.push_str(&format!("&cacheto={}", urlencoded(&json)));
		}
		for (name, value) in build.additional_contexts() {
			let mapped = map_additional_context(&self.base_dir, &value);
			qs.push_str(&format!(
				"&additionalbuildcontexts={}",
				urlencoded(&format!("{name}={mapped}"))
			));
		}
		if !build.ssh().is_empty() {
			warn!(
				"build.ssh is not supported over the libpod REST build API; ignoring {:?}",
				build.ssh()
			);
		}

		if matches!(body_plan, BodyPlan::Empty) {
			qs.push_str(&format!("&remote={}", urlencoded(&context_str)));
		}

		let path = format!("{API_PREFIX}/build?{qs}");
		let resp = match body_plan {
			BodyPlan::Empty => self
				.client
				.post_bytes_stream(&path, Bytes::new(), "application/x-tar")
				.await
				.map_err(ComposeError::Podman)?,
			BodyPlan::Stream {
				context,
				source,
				secrets,
			} => {
				// The tar writer runs on a blocking thread feeding the request
				// body; drain the body first, then join the writer, since joining
				// before the body is drained would deadlock on the bounded
				// channel. If the request itself failed, that transport error is
				// the real cause; otherwise surface any context-assembly error.
				let (producer, body) = context_body(context, source, secrets);
				let sent = self
					.client
					.post_stream_body(&path, body, "application/x-tar")
					.await;
				let produced = producer
					.await
					.map_err(|e| ComposeError::Build(e.to_string()))?;
				match sent {
					Ok(resp) => {
						produced?;
						resp
					}
					Err(e) => return Err(ComposeError::Podman(e)),
				}
			}
		};
		let mut stream = crate::libpod::parse_json_lines::<BuildOutput, _>(resp.into_body());

		// Open the board row for this image before the first `STEP` line, so
		// the row's `Building` verb is what the reader sees while the stream
		// arrives. `progress::start` is a no-op when the board already had
		// the row seeded (`build_all_with_options` did this for a standalone
		// `build`), and inserts it before the service's container row when
		// `up --build` calls into a board the `up` pass opened. Either way,
		// the row ends up with the right identifier, in the right position
		// and with a working verb (#1681).
		if !opts.quiet {
			let first_container = self.replica_names(service_name, service).into_iter().next();
			match first_container {
				Some(container_name) => crate::ui::progress::start_anchored(
					"Image",
					&tag,
					"Building",
					Some("Container"),
					Some(&container_name),
				),
				None => crate::ui::progress::start("Image", &tag, "Building"),
			}
		}

		// The captured stream of this image: needed on a terminal failure
		// path, where the notes buffer only kept the last 4 lines and the
		// rest has to be replayed as scrollback so the reason is on screen.
		// Off a terminal, every line is already in stderr through `note_for`,
		// so this stays empty in the test runs.
		let mut capture: Vec<String> = Vec::new();
		// Track the last image id the stream emitted, so the success path
		// can put it on stdout when stdout is not a terminal. Buildah
		// closes with a `--> sha256:<64-hex>` line, which is what a script
		// capturing stdout wants; on a terminal it is dropped (the row
		// already says it landed). `None` until the first matching line.
		let mut last_image_id: Option<String> = None;
		let mut progress = BuildStreamProgress::new();

		while let Some(result) = stream.next().await {
			match result {
				Ok(output) => {
					if !output.stream.is_empty() {
						let trimmed = output.stream.trim_end().to_string();
						if !opts.quiet && !trimmed.is_empty() {
							// `STEP n/m:` lines advance the row verb. Every
							// other line is a tail note, painted dimmed under
							// the row on a terminal and prefixed on stderr in
							// a pipe.
							if let Some(verb) = progress.observe(&trimmed) {
								crate::ui::progress::start("Image", &tag, &verb);
							}
							// The image id line is the one carry-over from
							// today that a script reading stdout needs. It
							// goes to notes/live as any other line, and is
							// additionally stashed so the success path can
							// echo it to stdout when stdout is not a tty.
							if let Some(id) = parse_image_id_line(&trimmed) {
								last_image_id = Some(id);
							}
							crate::ui::progress::note_for("Image", &tag, &trimmed);
						}
						capture.push(trimmed);
					}
					if let Some(err) = output.error_detail.and_then(|e| e.message) {
						return Err(self.fail_build(&tag, err, capture, opts.quiet).await);
					}
					if let Some(err) = output.error {
						if !err.is_empty() {
							return Err(self.fail_build(&tag, err, capture, opts.quiet).await);
						}
					}
				}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}

		// Success: close the row with `Built`. The trailing image id goes to
		// stdout only when stdout is not a terminal, so a script reading
		// `podup build | awk '{print $1}'` can still pluck the id; on a
		// terminal the row says it landed and the id would only be noise.
		if !opts.quiet {
			crate::ui::progress_line("Image", &tag, "Built");
		}
		if !std::io::stdout().is_terminal() {
			let resolved = match last_image_id {
				Some(id) => Some(id),
				None => match self.image_id(&tag).await {
					Ok(Some(id)) => Some(id),
					_ => None,
				},
			};
			if let Some(id) = resolved {
				use std::io::Write;
				let _ = writeln!(std::io::stdout(), "{id}");
			}
		}

		self.apply_extra_tags(build, &tag, &wire_tag).await?;
		Ok(())
	}
}
