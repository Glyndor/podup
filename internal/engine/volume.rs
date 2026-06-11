//! Volume creation and secret/config materialisation.
//!
//! [`Engine::create_volumes`] pre-creates named volumes before containers start.
//! [`Engine::build_secret_binds`] and [`Engine::build_config_binds`] materialise
//! inline secrets/configs to a restricted temp directory. Bind-string and
//! Mount-API helpers live in [`super::volume_mounts`].

use std::collections::HashMap;

use tracing::info;

use crate::compose::types::{
	ComposeFile, ConfigConfig, SecretConfig, Service, ServiceConfigRef, ServiceSecretRef,
};
use crate::error::{ComposeError, Result};
use crate::libpod::types::volume::VolumeCreateOptions;

use super::{staging, Engine};

impl Engine {
	pub(super) async fn create_volumes(&self, file: &ComposeFile) -> Result<()> {
		for (name, config) in &file.volumes {
			let external = config.as_ref().and_then(|c| c.external).unwrap_or(false);
			if external {
				continue;
			}

			let volume_name = config
				.as_ref()
				.and_then(|c| c.name.as_deref())
				.map(|s| s.to_string())
				.unwrap_or_else(|| format!("{}_{}", self.project, name));

			let mut labels: HashMap<String, String> = config
				.as_ref()
				.map(|c| c.labels.to_map())
				.unwrap_or_default();
			labels.insert("podup.project".to_string(), self.project.clone());

			let driver = config
				.as_ref()
				.and_then(|c| c.driver.clone())
				.unwrap_or_else(|| "local".into());

			let driver_opts: HashMap<String, String> = config
				.as_ref()
				.map(|c| c.driver_opts.clone())
				.unwrap_or_default();

			let options = VolumeCreateOptions {
				name: Some(volume_name.clone()),
				driver: Some(driver),
				driver_opts,
				labels,
			};

			match self.client.post_json::<_, serde_json::Value>("/libpod/volumes/create", &options).await {
				Ok(_) => info!("created volume {volume_name}"),
				Err(ref e) if e.is_status(409) => {}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}
		Ok(())
	}

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
						binds.push(format!("{host_path}:{target}:ro"));
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
					SecretConfig {
						external: Some(true),
						..
					} => {
						tracing::debug!("external secret {name} — relying on runtime injection");
					}
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
						binds.push(format!("{host_path}:{target}:ro"));
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
					ConfigConfig {
						external: Some(true),
						..
					} => {
						tracing::debug!("external config {name} — relying on runtime injection");
					}
					_ => {}
				}
			}
		}
		Ok(binds)
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

	pub(super) fn cleanup_temp_dir(&self) {
		if let Ok(dir) = self.staging_dir() {
			let _ = std::fs::remove_dir_all(dir);
		}
	}

	fn staging_dir(&self) -> Result<std::path::PathBuf> {
		if !staging::is_safe_project_name(&self.project) {
			return Err(ComposeError::Unsupported(format!(
				"project name must be ASCII alphanumeric/dash/underscore/dot \
				 and must not start with a dot: {}",
				self.project
			)));
		}
		Ok(staging::staging_base()?.join(&self.project))
	}
}
