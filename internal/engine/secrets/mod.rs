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
//!   staging directory.
//! * `external: true` → mapped to a pre-existing `podman secret`, preflighted
//!   with [`Engine::ensure_external_exists`] so a missing secret fails closed.
//!
//! The pure compose→plan mapping lives in [`plan`].

mod plan;

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
	/// `content:`/`environment:` sources are created on the daemon under a
	/// project-scoped name; `external: true` sources are preflighted for
	/// existence so a missing secret fails closed instead of starting a
	/// container that lacks it. `file:` sources are handled as bind mounts.
	pub(super) async fn build_native_secrets(
		&self,
		service: &Service,
		file: &ComposeFile,
	) -> Result<Vec<Secret>> {
		let plans = collect_native_plans(&self.project, service, file)?;
		let mut secrets = Vec::with_capacity(plans.len());
		for plan in plans {
			match &plan.payload {
				Some(bytes) => self.create_secret(&plan.source, bytes).await?,
				None => {
					self.ensure_external_exists("secret", "secrets", &plan.source)
						.await?
				}
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

	/// Create a Podman-native secret named `name` holding `payload`, labelled
	/// `podup.project=<proj>` so it can be cleaned up on `down`. The payload size
	/// is checked up front to turn Podman's opaque 500 into a clear message.
	///
	/// Idempotent across re-`up`s via a single atomic `replace=true` create: it
	/// overwrites an existing secret of the name and also succeeds when none
	/// exists, so no separate delete is needed.
	async fn create_secret(&self, name: &str, payload: &[u8]) -> Result<()> {
		check_secret_size(name, payload.len())?;
		let labels = serde_json::json!({ "podup.project": self.project }).to_string();
		let path = format!(
			"{API_PREFIX}/secrets/create?name={}&replace=true&labels={}",
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
		Ok(())
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
