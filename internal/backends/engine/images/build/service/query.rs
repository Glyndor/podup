//! Build arguments, labels and the libpod query string.

use tracing::warn;

use crate::compose::types::BuildConfig;
use crate::error::{ComposeError, Result};
use crate::libpod::urlencoded;
use crate::libpod::validate::pre_validate_build;
use crate::size;

use super::super::names::looks_like_secret;
use super::super::{context::map_additional_context, BuildOptions, Engine};
use super::cache::{references, CacheField};

impl Engine {
	pub(super) async fn service_build_query(
		&self,
		service_name: &str,
		build: &BuildConfig,
		opts: &BuildOptions,
		tag: &str,
		dockerfile_name: &str,
		secret_specs: &[String],
	) -> Result<(String, String)> {
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
			warn!(target: "podup::backends::engine::images::build::service",
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
					warn!(target: "podup::backends::engine::images::build::service","build.platforms: libpod builds one platform per request; building {first}, ignoring {rest_count} other(s)");
				}
			}).cloned(),
			_ => None,
		};
		// Distinguish "absent" from "present but unparseable": a malformed
		// `shm_size` must be rejected, not silently dropped to the default.
		//
		// The byte count stays `i64`, the width libpod's `shmsize` query takes
		// and the width the container spec uses. It used to be narrowed to
		// `i32` on the way into the query, so any size above 2 GiB was
		// truncated: 4 GiB became 0 and 3 GiB became a negative number, and
		// the container was built with the wrong shared-memory size.
		let shmsize = match build.shm_size() {
			Some(raw) => Some(size::parse_memory(raw).ok_or_else(|| {
				ComposeError::Build(format!("invalid build.shm_size value '{raw}'"))
			})?),
			None => None,
		};
		// The libpod endpoint reads `extrahosts` as a JSON array of
		// `host:ip` entries. A comma-separated list is not that shape: the
		// daemon rejects the build before it starts.
		let extrahosts = if build.extra_hosts().is_empty() {
			None
		} else {
			Some(crate::engine::to_query_json(
				"build.extra_hosts",
				&build.extra_hosts(),
			)?)
		};
		// The endpoint reads the caches as a JSON array of image references.
		// Compose writes them with the global cache syntax, so a registry
		// cache travels as its reference and a cache type with no libpod
		// equivalent is left out.
		let cache_from = references(CacheField::From, build.cache_from());
		let cachefrom = if cache_from.is_empty() {
			None
		} else {
			Some(crate::engine::to_query_json(
				"build.cache_from",
				&cache_from,
			)?)
		};
		let buildargs_json = if build_args.is_empty() {
			None
		} else {
			Some(crate::engine::to_query_json("build.args", &build_args)?)
		};
		let labels_json = if labels.is_empty() {
			None
		} else {
			Some(crate::engine::to_query_json("build.labels", &labels)?)
		};
		let build_ulimits = super::super::render_build_ulimits(build);
		let ulimits_json = if build_ulimits.is_empty() {
			None
		} else {
			Some(crate::engine::to_query_json(
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
		let wire_tag = self.normalize_image_reference(tag).await?;
		let mut qs = format!(
			"t={}&rm=true&forcerm=true&layers=true&nocache={}&outputformat={}",
			urlencoded(&wire_tag),
			build.no_cache() || opts.no_cache,
			urlencoded("application/vnd.docker.distribution.manifest.v2+json"),
		);
		qs.push_str(&format!("&dockerfile={}", urlencoded(dockerfile_name)));
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
			let json = crate::engine::to_query_json("build.secrets", &secret_specs)?;
			qs.push_str(&format!("&secrets={}", urlencoded(&json)));
		}
		let cache_to = references(CacheField::To, build.cache_to());
		if !cache_to.is_empty() {
			let json = crate::engine::to_query_json("build.cache_to", &cache_to)?;
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
			warn!(target: "podup::backends::engine::images::build::service",
				"build.ssh is not supported over the libpod REST build API; ignoring {:?}",
				build.ssh()
			);
		}

		Ok((qs, wire_tag))
	}
}
