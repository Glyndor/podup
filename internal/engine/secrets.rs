//! Secret and config materialisation.
//!
//! [`Engine::build_secret_binds`] and [`Engine::build_config_binds`] materialise
//! `file:`, inline `content:` and `environment:` secret/config sources into a
//! restricted temp directory and return their bind strings.
//! [`Engine::build_native_secrets`] maps `external: true` secrets/configs to
//! Podman-native secrets attached to the container create spec.

use crate::compose::types::{
	ComposeFile, ConfigConfig, SecretConfig, Service, ServiceConfigRef, ServiceSecretRef,
};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::Secret;

use super::{staging, Engine};

impl Engine {
	pub(super) fn build_secret_binds(
		&self,
		service: &Service,
		file: &ComposeFile,
	) -> Result<Vec<String>> {
		let mut binds = Vec::new();
		for secret_ref in &service.secrets {
			let (name, target_override, ref_mode, ref_uid, ref_gid) = match secret_ref {
				ServiceSecretRef::Short(s) => (s.clone(), None, None, None, None),
				ServiceSecretRef::Long {
					source,
					target,
					mode,
					uid,
					gid,
				} => (
					source.clone(),
					target.clone(),
					*mode,
					uid.clone(),
					gid.clone(),
				),
			};
			if let Some(config) = file.secrets.get(&name) {
				let target = target_override.unwrap_or_else(|| format!("/run/secrets/{name}"));
				match config {
					SecretConfig {
						file: Some(host_path),
						..
					} => {
						// Resolve like a bind-mount source: a relative `file:` is
						// anchored to the project dir (not the Podman service's cwd)
						// and `~` is expanded — same handling as `volumes:`, which
						// already mount arbitrary host paths.
						let resolved =
							super::container::resolve_bind_source(host_path, &self.base_dir);
						binds.push(format!("{resolved}:{target}:ro"));
					}
					SecretConfig {
						content: Some(content),
						..
					} => {
						let path = self.materialize_inline_full(
							"secrets",
							&name,
							content.as_bytes(),
							ref_mode,
							ref_uid.as_deref(),
							ref_gid.as_deref(),
						)?;
						binds.push(format!("{}:{target}:ro", path.display()));
					}
					SecretConfig {
						environment: Some(env_var),
						..
					} => {
						let value = std::env::var(env_var).map_err(|_| {
							ComposeError::Unsupported(format!(
								"secret '{name}' references env var '{env_var}' which is not set"
							))
						})?;
						let path = self.materialize_inline_full(
							"secrets",
							&name,
							value.as_bytes(),
							ref_mode,
							ref_uid.as_deref(),
							ref_gid.as_deref(),
						)?;
						binds.push(format!("{}:{target}:ro", path.display()));
					}
					// `external: true` secrets are injected natively — see
					// build_native_secrets — not as bind mounts, so skip them here.
					_ => {}
				}
			}
		}
		Ok(binds)
	}

	pub(super) fn build_config_binds(
		&self,
		service: &Service,
		file: &ComposeFile,
	) -> Result<Vec<String>> {
		let mut binds = Vec::new();
		for config_ref in &service.configs {
			let (name, target_override, ref_mode, ref_uid, ref_gid) = match config_ref {
				ServiceConfigRef::Short(s) => (s.clone(), None, None, None, None),
				ServiceConfigRef::Long {
					source,
					target,
					mode,
					uid,
					gid,
				} => (
					source.clone(),
					target.clone(),
					*mode,
					uid.clone(),
					gid.clone(),
				),
			};
			if let Some(cfg) = file.configs.get(&name) {
				let target = target_override.unwrap_or_else(|| format!("/{name}"));
				match cfg {
					ConfigConfig {
						file: Some(host_path),
						..
					} => {
						// Resolve like a bind-mount source: anchor a relative path to
						// the project dir and expand `~`, matching `volumes:` handling.
						let resolved =
							super::container::resolve_bind_source(host_path, &self.base_dir);
						binds.push(format!("{resolved}:{target}:ro"));
					}
					ConfigConfig {
						content: Some(content),
						..
					} => {
						let path = self.materialize_inline_full(
							"configs",
							&name,
							content.as_bytes(),
							ref_mode,
							ref_uid.as_deref(),
							ref_gid.as_deref(),
						)?;
						binds.push(format!("{}:{target}:ro", path.display()));
					}
					ConfigConfig {
						environment: Some(env_var),
						..
					} => {
						let value = std::env::var(env_var).map_err(|_| {
							ComposeError::Unsupported(format!(
								"config '{name}' references env var '{env_var}' which is not set"
							))
						})?;
						let path = self.materialize_inline_full(
							"configs",
							&name,
							value.as_bytes(),
							ref_mode,
							ref_uid.as_deref(),
							ref_gid.as_deref(),
						)?;
						binds.push(format!("{}:{target}:ro", path.display()));
					}
					// `external: true` configs are injected natively — see
					// build_native_secrets — not as bind mounts, so skip them here.
					_ => {}
				}
			}
		}
		Ok(binds)
	}

	/// Build the Podman-native secret references for a service: every `external:
	/// true` secret and config it references, mapped to an existing `podman
	/// secret` and mounted into the container. Each source is preflighted with
	/// [`Engine::ensure_external_exists`] so a missing secret fails closed with a
	/// clear message instead of starting a container that lacks it.
	pub(super) async fn build_native_secrets(
		&self,
		service: &Service,
		file: &ComposeFile,
	) -> Result<Vec<Secret>> {
		let secrets = collect_native_secrets(service, file)?;
		for secret in &secrets {
			self.ensure_external_exists("secret", "secrets", &secret.source)
				.await?;
		}
		Ok(secrets)
	}

	fn materialize_inline_full(
		&self,
		kind: &str,
		name: &str,
		content: &[u8],
		mode: Option<u32>,
		uid: Option<&str>,
		gid: Option<&str>,
	) -> Result<std::path::PathBuf> {
		if std::path::Path::new(name)
			.components()
			.any(|c| !matches!(c, std::path::Component::Normal(_)))
		{
			return Err(ComposeError::Unsupported(format!(
				"{kind} name must not contain path separators or '..': {name}"
			)));
		}

		let dir = self.staging_dir()?.join(kind);
		staging::create_private_subdir(&dir)?;

		let path = dir.join(name);
		staging::write_private_file(&path, content)?;

		if let Some(m) = mode {
			staging::apply_mode(&path, m)?;
		}
		staging::apply_owner(&path, uid, gid);

		Ok(path)
	}
}

/// Collect the Podman-native secret specs for a service without touching the
/// daemon: every `external: true` secret and config it references becomes a
/// [`Secret`] pointing at an existing `podman secret`. Pure, so the mapping is
/// unit-testable; [`Engine::build_native_secrets`] adds the existence preflight.
///
/// A dangerous `mode:` (execute, setuid, setgid or sticky bits) on any
/// reference is rejected here — the same class of check `apply_mode` enforces
/// for the bind-mount path — so a hostile mode never reaches the daemon.
fn collect_native_secrets(service: &Service, file: &ComposeFile) -> Result<Vec<Secret>> {
	let mut secrets = Vec::new();

	for secret_ref in &service.secrets {
		let (name, target_override, mode, uid, gid) = match secret_ref {
			ServiceSecretRef::Short(s) => (s.clone(), None, None, None, None),
			ServiceSecretRef::Long {
				source,
				target,
				mode,
				uid,
				gid,
			} => (
				source.clone(),
				target.clone(),
				*mode,
				uid.clone(),
				gid.clone(),
			),
		};
		if let Some(def) = file.secrets.get(&name) {
			if def.external == Some(true) {
				let source = def.name.clone().unwrap_or_else(|| name.clone());
				// Default the mount filename to the compose name (so apps still
				// find /run/secrets/<name> even when the Podman secret is named
				// differently); a long-form target overrides it.
				let target = target_override.unwrap_or(name);
				secrets.push(native_secret(source, target, mode, uid, gid)?);
			}
		}
	}

	for config_ref in &service.configs {
		let (name, target_override, mode, uid, gid) = match config_ref {
			ServiceConfigRef::Short(s) => (s.clone(), None, None, None, None),
			ServiceConfigRef::Long {
				source,
				target,
				mode,
				uid,
				gid,
			} => (
				source.clone(),
				target.clone(),
				*mode,
				uid.clone(),
				gid.clone(),
			),
		};
		if let Some(def) = file.configs.get(&name) {
			if def.external == Some(true) {
				let source = def.name.clone().unwrap_or_else(|| name.clone());
				// Configs default to an absolute container-root path, matching the
				// bind-mount config behaviour; an absolute target is used as-is.
				let target = target_override.unwrap_or_else(|| format!("/{name}"));
				secrets.push(native_secret(source, target, mode, uid, gid)?);
			}
		}
	}

	Ok(secrets)
}

/// Assemble one [`Secret`] from a compose reference. A dangerous `mode:`
/// (execute/setuid/setgid/sticky) is rejected before the spec is built so it
/// never reaches Podman. `uid`/`gid` are numeric in libpod, so non-numeric
/// values (a user/group name) are dropped to the default.
fn native_secret(
	source: String,
	target: String,
	mode: Option<u32>,
	uid: Option<String>,
	gid: Option<String>,
) -> Result<Secret> {
	if let Some(m) = mode {
		staging::reject_dangerous_secret_mode(m, &source)?;
	}
	Ok(Secret {
		source,
		target: Some(target),
		uid: uid.and_then(|s| s.parse().ok()),
		gid: gid.and_then(|s| s.parse().ok()),
		mode,
	})
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
		// A relative `file:` must resolve against the project dir, not the
		// Podman service's cwd — same as a bind-mount source. Build the expected
		// path via `Path::join` so the separator is correct on every platform.
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
		// Absolute paths are honored unchanged (compose allows mounting any host
		// file, exactly as `volumes:` does). Uses a Unix-absolute path, so the
		// assertion is gated to Unix.
		let yaml = "services:\n  web:\n    image: nginx\n    configs: [cfg]\nconfigs:\n  cfg:\n    file: /etc/app/cfg.yaml\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let engine = engine_with_base("/srv/project");
		let binds = engine
			.build_config_binds(&file.services["web"], &file)
			.unwrap();
		assert_eq!(binds, vec!["/etc/app/cfg.yaml:/cfg:ro"]);
	}

	#[test]
	fn external_secret_becomes_native_targeting_compose_name() {
		// An `external: true` secret maps to a Podman-native secret. With no
		// custom name the source equals the compose name, and the mount filename
		// defaults to that name so apps still read /run/secrets/<name>.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let secrets = collect_native_secrets(&file.services["web"], &file).unwrap();
		assert_eq!(secrets.len(), 1);
		assert_eq!(secrets[0].source, "tok");
		assert_eq!(secrets[0].target.as_deref(), Some("tok"));
		assert!(secrets[0].uid.is_none() && secrets[0].gid.is_none() && secrets[0].mode.is_none());
	}

	#[test]
	fn external_secret_long_form_maps_source_target_and_perms() {
		// A long-form ref overrides the mount name, and a custom top-level `name:`
		// is the actual Podman secret to look up. Numeric uid/gid/mode pass through.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        target: app_tok\n        uid: \"100\"\n        gid: \"101\"\n        mode: 256\nsecrets:\n  tok:\n    external: true\n    name: real_tok\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let secrets = collect_native_secrets(&file.services["web"], &file).unwrap();
		assert_eq!(secrets.len(), 1);
		let s = &secrets[0];
		assert_eq!(s.source, "real_tok");
		assert_eq!(s.target.as_deref(), Some("app_tok"));
		assert_eq!(s.uid, Some(100));
		assert_eq!(s.gid, Some(101));
		assert_eq!(s.mode, Some(256));
	}

	#[test]
	fn external_config_becomes_native_with_absolute_default_target() {
		// Configs default to an absolute container-root path, matching the
		// bind-mount config behaviour; an absolute target is mounted as-is.
		let yaml = "services:\n  web:\n    image: nginx\n    configs: [cfg]\nconfigs:\n  cfg:\n    external: true\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let secrets = collect_native_secrets(&file.services["web"], &file).unwrap();
		assert_eq!(secrets.len(), 1);
		assert_eq!(secrets[0].source, "cfg");
		assert_eq!(secrets[0].target.as_deref(), Some("/cfg"));
	}

	#[test]
	fn non_external_secret_is_not_a_native_secret() {
		// A `file:` secret is materialised as a bind mount, not a native secret.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    file: ./tok.txt\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let secrets = collect_native_secrets(&file.services["web"], &file).unwrap();
		assert!(secrets.is_empty());
	}

	#[test]
	fn non_numeric_uid_drops_to_default() {
		// libpod secret uid/gid are numeric; a user/group name has no native
		// equivalent here and falls back to the default rather than erroring.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        uid: appuser\nsecrets:\n  tok:\n    external: true\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let secrets = collect_native_secrets(&file.services["web"], &file).unwrap();
		assert_eq!(secrets.len(), 1);
		assert!(secrets[0].uid.is_none());
	}

	#[test]
	fn native_secret_rejects_setuid_mode() {
		// A hostile compose file requesting setuid on an external secret must be
		// refused before the spec reaches Podman — the same guard the bind-mount
		// path enforces. 0o4000 is setuid.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        mode: 2048\nsecrets:\n  tok:\n    external: true\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		assert!(collect_native_secrets(&file.services["web"], &file).is_err());
	}

	#[test]
	fn native_secret_rejects_execute_mode() {
		// 0o777 (= 511) sets execute bits; a secret holds data, never code.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        mode: 511\nsecrets:\n  tok:\n    external: true\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		assert!(collect_native_secrets(&file.services["web"], &file).is_err());
	}

	#[test]
	fn native_config_rejects_setgid_mode() {
		// External configs share the same mode guard. 0o2000 (= 1024) is setgid.
		let yaml = "services:\n  web:\n    image: nginx\n    configs:\n      - source: cfg\n        mode: 1024\nconfigs:\n  cfg:\n    external: true\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		assert!(collect_native_secrets(&file.services["web"], &file).is_err());
	}

	#[test]
	fn native_secret_allows_world_readable_mode() {
		// 0o444 (= 292, world-readable) is the Podman/compose default for a
		// native secret materialised inside the container — it must be allowed,
		// unlike the shared-host bind-mount path which rejects o+r.
		let yaml = "services:\n  web:\n    image: nginx\n    secrets:\n      - source: tok\n        mode: 292\nsecrets:\n  tok:\n    external: true\n";
		let file = crate::compose::parse_str_raw(yaml).unwrap();
		let secrets = collect_native_secrets(&file.services["web"], &file).unwrap();
		assert_eq!(secrets[0].mode, Some(0o444));
	}
}
