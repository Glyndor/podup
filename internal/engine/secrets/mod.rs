//! Secret and config injection.
//!
//! `file:` secret/config sources are bind-mounted read-only from the host —
//! the file already lives there, so no copy is made. Inline `content:` and
//! `environment:` sources, and `external: true` references, are injected as
//! Podman-native secrets attached to the container create spec:
//!
//! * inline `content:`/`environment:` → created over the libpod API
//!   (`secrets/create`, removing any prior secret of the name first so a re-`up`
//!   is idempotent) under a project-scoped name, so nothing is written to a host
//!   staging directory. The project's whole inline union is created once up
//!   front by [`Engine::create_inline_secrets`] (before services start
//!   concurrently), not per-service, so a shared name is never raced.
//! * `external: true` → mapped to a pre-existing `podman secret`, preflighted
//!   with [`Engine::ensure_external_exists`] so a missing secret fails closed.
//!
//! The pure compose→plan mapping lives in [`plan`].

mod plan;

use std::collections::HashMap;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::Secret;
use crate::libpod::{urlencoded, API_PREFIX};

use plan::{
	check_secret_size, collect_native_plans, is_inline_source, ref_name_target, scoped_name,
};

use super::Engine;

impl Engine {
	/// Bind strings for `file:` secrets referenced by `service`. Inline and
	/// external secrets are injected natively (see [`Engine::build_native_secrets`])
	/// and are skipped here.
	pub(super) fn build_secret_binds(
		&self,
		service: &Service,
		file: &ComposeFile,
	) -> Result<Vec<String>> {
		let mut binds = Vec::new();
		for secret_ref in &service.secrets {
			let (name, target_override) = ref_name_target(secret_ref.source(), secret_ref.target());
			if let Some(def) = file.secrets.get(&name) {
				if let Some(host_path) = &def.file {
					let target = target_override.unwrap_or_else(|| format!("/run/secrets/{name}"));
					// Resolve like a bind-mount source: a relative `file:` is anchored
					// to the project dir (not the Podman service's cwd) and `~` is
					// expanded — same handling as `volumes:`.
					let resolved = super::container::resolve_bind_source(host_path, &self.base_dir);
					binds.push(make_bind(&name, &resolved, &target)?);
				}
			}
		}
		Ok(binds)
	}

	/// Bind strings for `file:` configs referenced by `service`. Inline and
	/// external configs are injected natively and are skipped here.
	pub(super) fn build_config_binds(
		&self,
		service: &Service,
		file: &ComposeFile,
	) -> Result<Vec<String>> {
		let mut binds = Vec::new();
		for config_ref in &service.configs {
			let (name, target_override) = ref_name_target(config_ref.source(), config_ref.target());
			if let Some(def) = file.configs.get(&name) {
				if let Some(host_path) = &def.file {
					let target = target_override.unwrap_or_else(|| format!("/{name}"));
					let resolved = super::container::resolve_bind_source(host_path, &self.base_dir);
					binds.push(make_bind(&name, &resolved, &target)?);
				}
			}
		}
		Ok(binds)
	}

	/// Build the Podman-native secret references for a service. Inline
	/// `content:`/`environment:` sources must already have been created by
	/// [`Engine::create_inline_secrets`] (run once up front), so this only
	/// preflights `external: true` sources for existence — failing closed
	/// rather than starting a container that lacks the secret — and assembles
	/// the per-service references attached to the container spec. `file:`
	/// sources are handled as bind mounts.
	///
	/// Creation is deliberately *not* done here: services in the same
	/// dependency level are brought up concurrently, and a per-service
	/// delete-then-create on a shared inline secret name would race (one create
	/// could clobber a secret another service's container is about to use). The
	/// up-front pass creates each inline secret exactly once instead.
	pub(super) async fn build_native_secrets(
		&self,
		service: &Service,
		file: &ComposeFile,
	) -> Result<Vec<Secret>> {
		let plans = collect_native_plans(&self.project, service, file)?;
		let mut secrets = Vec::with_capacity(plans.len());
		for plan in plans {
			// Inline payloads are created up front; only external sources need a
			// (read-only, idempotent) existence preflight here.
			if plan.payload.is_none() {
				self.ensure_external_exists("secret", "secrets", &plan.source)
					.await?;
			}
			secrets.push(Secret {
				source: plan.source,
				target: Some(plan.target),
				uid: plan.uid,
				gid: plan.gid,
				mode: plan.mode,
			});
		}
		Ok(secrets)
	}

	/// Create the union of inline `content:`/`environment:` secrets and configs
	/// declared across *all* services in the project, once, before the
	/// per-level start loop — mirroring how [`Engine::create_networks`] and
	/// [`Engine::create_volumes`] pre-create their resources.
	///
	/// Doing this up front fixes the race in which two services in the same
	/// dependency level (started concurrently) both ran the non-atomic
	/// delete-then-create for the same project-scoped secret name, so one could
	/// delete the secret the other had just created. The same scoped name is
	/// created exactly once here (later services share it), and each created
	/// secret carries the `podup.project=<proj>` label so the label-guarded
	/// teardown on `down` still only removes secrets podup owns.
	pub(super) async fn create_inline_secrets(&self, file: &ComposeFile) -> Result<()> {
		for (name, bytes) in collect_inline_union(&self.project, file)? {
			self.create_secret(&name, &bytes).await?;
		}
		Ok(())
	}

	/// Create a Podman-native secret named `name` holding `payload`, labelled
	/// `podup.project=<proj>` so it can be cleaned up on `down`. The payload size
	/// is checked up front to turn Podman's opaque 500 into a clear message.
	///
	/// Idempotent across re-`up`s: rather than `replace=true` (which some Podman
	/// 5.x builds reject when the secret does not yet exist — the internal delete
	/// fails with "no secret data with ID"), the existing secret of this name is
	/// removed first (a 404 is fine) and then created fresh.
	async fn create_secret(&self, name: &str, payload: &[u8]) -> Result<()> {
		check_secret_size(name, payload.len())?;
		// Guard the delete-then-create: if a secret of this name already exists and
		// is not labelled as ours, refuse rather than clobber a foreign secret.
		// Our own secret (or a 404) is replaced fresh, keeping re-`up` idempotent.
		let inspect = format!("{API_PREFIX}/secrets/{}/json", urlencoded(name));
		match self.client.get_json::<serde_json::Value>(&inspect).await {
			Ok(info) => {
				let owned = info
					.get("Spec")
					.and_then(|spec| spec.get("Labels"))
					.and_then(|labels| labels.get("podup.project"))
					.and_then(|v| v.as_str())
					== Some(self.project.as_str());
				if !owned {
					return Err(ComposeError::Unsupported(format!(
						"a secret named '{name}' already exists and is not labelled \
						 podup.project={} — refusing to overwrite a secret podup did \
						 not create",
						self.project
					)));
				}
			}
			Err(e) if e.is_status(404) => {}
			Err(e) => return Err(ComposeError::Podman(e)),
		}
		let delete_path = format!("{API_PREFIX}/secrets/{}", urlencoded(name));
		self.client
			.delete_ok(&delete_path)
			.await
			.map_err(ComposeError::Podman)?;
		let labels = serde_json::json!({ "podup.project": self.project }).to_string();
		let path = format!(
			"{API_PREFIX}/secrets/create?name={}&labels={}",
			urlencoded(name),
			urlencoded(&labels),
		);
		// The response is `{"ID": "..."}`; we don't need the id, only success.
		self.client
			.post_bytes_json::<serde_json::Value>(
				&path,
				bytes::Bytes::copy_from_slice(payload),
				"application/octet-stream",
			)
			.await
			.map(|_| ())
			.map_err(ComposeError::Podman)
	}

	/// Remove the project-scoped native secrets created on `up` for inline
	/// `content:`/`environment:` secrets and configs, mirroring the volume and
	/// network teardown on `down`. `external:` and `file:` references own no
	/// podup-created secret and are left untouched; a missing secret is ignored
	/// (`delete_ok` swallows a 404). Best-effort: a delete failure is logged, not
	/// fatal, so the rest of teardown proceeds.
	pub(super) async fn remove_internal_secrets(&self, file: &ComposeFile) -> Result<()> {
		for (name, def) in &file.secrets {
			if is_inline_source(
				def.external,
				def.content.as_deref(),
				def.environment.as_deref(),
			) {
				self.delete_secret(&scoped_name(&self.project, "secret", name))
					.await;
			}
		}
		for (name, def) in &file.configs {
			if is_inline_source(
				def.external,
				def.content.as_deref(),
				def.environment.as_deref(),
			) {
				self.delete_secret(&scoped_name(&self.project, "config", name))
					.await;
			}
		}
		// Catch orphans: a secret podup created on a previous `up` whose compose key
		// was since renamed/removed (or a `down` run without the original file) is
		// not reached by the loops above. Sweep every secret carrying this project's
		// label and remove it, so no podup-created secret is left behind.
		for name in self.list_project_secret_names().await {
			self.delete_secret(&name).await;
		}
		Ok(())
	}

	/// Names of all native secrets labelled `podup.project=<proj>` — the secrets
	/// podup created for this project. libpod's `/secrets/json` rejects a `label`
	/// filter (HTTP 500 `invalid filter "label"`), so the full list is fetched and
	/// filtered client-side by the `podup.project` label. Best-effort: a list
	/// failure yields an empty set so teardown still proceeds via the
	/// compose-driven deletes above.
	async fn list_project_secret_names(&self) -> Vec<String> {
		let path = format!("{API_PREFIX}/secrets/json");
		match self.client.get_json::<Vec<serde_json::Value>>(&path).await {
			Ok(list) => list
				.iter()
				.filter_map(|s| {
					let spec = s.get("Spec")?;
					let owned = spec
						.get("Labels")
						.and_then(|l| l.get("podup.project"))
						.and_then(|v| v.as_str())
						== Some(self.project.as_str());
					if owned {
						spec.get("Name")
							.and_then(|n| n.as_str())
							.map(str::to_string)
					} else {
						None
					}
				})
				.collect(),
			Err(e) => {
				tracing::debug!("could not list project secrets for orphan cleanup: {e}");
				Vec::new()
			}
		}
	}

	/// Delete a project-scoped secret, but only after confirming it carries our
	/// `podup.project=<proj>` label — so a same-named secret the user created by
	/// hand (and which podup never created) is never destroyed on `down`. A
	/// missing secret (404) is a no-op.
	async fn delete_secret(&self, name: &str) {
		let inspect = format!("{API_PREFIX}/secrets/{}/json", urlencoded(name));
		match self.client.get_json::<serde_json::Value>(&inspect).await {
			Ok(info) => {
				let owned = info
					.get("Spec")
					.and_then(|spec| spec.get("Labels"))
					.and_then(|labels| labels.get("podup.project"))
					.and_then(|v| v.as_str())
					== Some(self.project.as_str());
				if !owned {
					tracing::warn!(
						"secret {name} is not labelled podup.project={} — \
						 leaving it untouched (not created by podup)",
						self.project
					);
					return;
				}
			}
			Err(e) if e.is_status(404) => return,
			Err(e) => {
				tracing::warn!("could not inspect secret {name} before removal: {e}");
				return;
			}
		}
		let path = format!("{API_PREFIX}/secrets/{}", urlencoded(name));
		match self.client.delete_ok(&path).await {
			Ok(()) => tracing::info!("removed secret {name}"),
			Err(e) => tracing::warn!("could not remove secret {name}: {e}"),
		}
	}
}

/// Build a `source:target:ro` bind string for a `file:` secret/config, rejecting
/// a colon in either the resolved host path or the target.
///
/// The bind string is later split with `splitn(3, ':')`; a stray colon in the
/// source or target shifts the field boundaries — redirecting the mount
/// destination, or merging the `:ro` flag into a malformed `rw:ro` token that
/// silently drops the read-only guarantee. A colon is not meaningful in a
/// container mount path, so reject it at the boundary instead of mis-parsing.
fn make_bind(name: &str, resolved: &str, target: &str) -> Result<String> {
	if resolved.contains(':') {
		return Err(ComposeError::Unsupported(format!(
			"secret/config '{name}': host path must not contain a colon: {resolved}"
		)));
	}
	if target.contains(':') {
		return Err(ComposeError::Unsupported(format!(
			"secret/config '{name}': mount target must not contain a colon: {target}"
		)));
	}
	Ok(format!("{resolved}:{target}:ro"))
}

/// Collect the project's inline `content:`/`environment:` secret/config
/// payloads, deduplicated by their scoped Podman secret name.
///
/// The same inline secret referenced by several services resolves to one
/// project-scoped name, so it is created once and shared. A first writer wins:
/// every reference to a given name yields the identical payload (the bytes come
/// from the single compose def), so the dedup is value-stable. No daemon access,
/// so the union and its dedup are unit-testable.
fn collect_inline_union(project: &str, file: &ComposeFile) -> Result<HashMap<String, Vec<u8>>> {
	let mut payloads: HashMap<String, Vec<u8>> = HashMap::new();
	for service in file.services.values() {
		for plan in collect_native_plans(project, service, file)? {
			if let Some(bytes) = plan.payload {
				payloads.entry(plan.source).or_insert(bytes);
			}
		}
	}
	Ok(payloads)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::libpod::Client;
	use std::path::PathBuf;

	fn engine_with_base(base: &str) -> Engine {
		Engine::with_base_dir(
			Client::new("unused"),
			"proj".to_string(),
			PathBuf::from(base),
		)
	}

	#[test]
	fn secret_file_relative_path_is_anchored_to_base_dir() {
		// A relative `file:` resolves against the project dir, not the Podman
		// service's cwd — same as a bind-mount source.
		let base = PathBuf::from("/srv/project");
		let yaml = "services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    file: secret.txt\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let engine = engine_with_base(&base.to_string_lossy());
		let binds = engine
			.build_secret_binds(&file.services["web"], &file)
			.unwrap();
		let expected = format!("{}:/run/secrets/tok:ro", base.join("secret.txt").display());
		assert_eq!(binds, vec![expected]);
	}

	#[cfg(unix)]
	#[test]
	fn config_file_absolute_path_is_passed_through() {
		// Absolute paths are honored unchanged, exactly as `volumes:` does.
		let yaml = "services:\n  web:\n    image: nginx\n    configs: [cfg]\nconfigs:\n  cfg:\n    file: /etc/app/cfg.yaml\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let engine = engine_with_base("/srv/project");
		let binds = engine
			.build_config_binds(&file.services["web"], &file)
			.unwrap();
		assert_eq!(binds, vec!["/etc/app/cfg.yaml:/cfg:ro"]);
	}

	#[test]
	fn make_bind_rejects_colon_in_path_or_target() {
		assert!(make_bind("s", "/host/a:b", "/run/secrets/s").is_err());
		assert!(make_bind("s", "/host/a", "/run/secrets/s:rw").is_err());
		assert_eq!(
			make_bind("s", "/host/a", "/run/secrets/s").unwrap(),
			"/host/a:/run/secrets/s:ro"
		);
	}

	#[test]
	fn inline_union_dedups_shared_secret_across_services() {
		// Two services in the same project both reference the same inline secret.
		// The up-front union must create it once (one scoped name), not once per
		// service — which is what previously raced delete-then-create.
		let yaml = "services:\n  a:\n    image: nginx\n    secrets: [tok]\n  b:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    content: shared\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let union = collect_inline_union("proj", &file).unwrap();
		assert_eq!(union.len(), 1);
		assert_eq!(
			union.get("proj_secret_tok").map(Vec::as_slice),
			Some(b"shared".as_slice())
		);
	}

	#[test]
	fn inline_union_collects_secrets_and_configs_skips_external_and_file() {
		// The union spans inline secrets and inline configs (distinct scoped
		// names), and excludes `external:` (podup never creates it) and `file:`
		// (a bind mount) sources.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets: [tok, ext, onfile]\n    configs: [cfg]\nsecrets:\n  tok:\n    content: s\n  ext:\n    external: true\n  onfile:\n    file: ./f.txt\nconfigs:\n  cfg:\n    content: c\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let union = collect_inline_union("proj", &file).unwrap();
		let mut names: Vec<&String> = union.keys().collect();
		names.sort();
		assert_eq!(names, vec!["proj_config_cfg", "proj_secret_tok"]);
	}

	#[test]
	fn inline_secret_produces_no_bind() {
		// An inline `content:` secret is injected natively, so it contributes no
		// bind string — only `file:` secrets do.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    content: data\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let engine = engine_with_base("/srv/project");
		let binds = engine
			.build_secret_binds(&file.services["web"], &file)
			.unwrap();
		assert!(binds.is_empty());
	}
}
