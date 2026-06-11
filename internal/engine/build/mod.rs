//! Image build and pull operations.
//!
//! [`Engine::pull_image`] fetches a pre-built image from a registry.
//! [`Engine::build_service`] compiles a build context tar, passes it to the
//! Podman libpod API, and applies any extra tags. Multi-stage targets are
//! passed as the `target=` query parameter — the full Dockerfile is always sent.

mod context;

use bytes::Bytes;
use futures::StreamExt;
use tracing::{debug, info, warn};

use crate::compose::types::{BuildConfig, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::{BuildOutput, ImagePullProgress};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;
use crate::size;

use context::{build_context_tar, build_context_tar_with_inline, map_additional_context};

use super::Engine;

/// Files to ship inside the build-context tar plus their matching `secrets=`
/// specs (`id=NAME,src=ENTRY`) for the libpod build endpoint.
type ResolvedBuildSecrets = (Vec<(String, Vec<u8>)>, Vec<String>);

impl Engine {
	pub(super) async fn pull_image(&self, service: &Service) -> Result<()> {
		let image = match &service.image {
			Some(img) => img.clone(),
			None => return Ok(()),
		};

		info!("pulling {image}");

		let pull_policy = match service.pull_policy.as_deref() {
			Some("always") => "always",
			Some("newer") => "newer",
			_ => "missing",
		};
		let mut query = format!("reference={}&policy={}", urlencoded(&image), pull_policy);
		if let Some(platform) = &service.platform {
			query.push_str(&format!("&platform={}", urlencoded(platform)));
		}

		let path = format!("{API_PREFIX}/images/pull?{query}");
		let resp = self
			.client
			.post_empty_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_json_lines::<ImagePullProgress>(resp.into_body());

		while let Some(result) = stream.next().await {
			match result {
				Ok(progress) => {
					if !progress.stream.is_empty() {
						debug!("{}", progress.stream.trim_end());
					}
					if !progress.error.is_empty() {
						warn!("pull error: {}", progress.error);
					}
				}
				Err(e) => warn!("pull warning: {e}"),
			}
		}

		Ok(())
	}

	/// Build (or rebuild) images for services that have a `build:` block.
	///
	/// If `target_services` is empty, every service with a build config is built.
	/// Services without a build config are silently skipped.
	pub async fn build_all(
		&self,
		file: &crate::compose::types::ComposeFile,
		target_services: &[String],
	) -> Result<()> {
		let names: Vec<String> = if target_services.is_empty() {
			file.services.keys().cloned().collect()
		} else {
			for name in target_services {
				if !file.services.contains_key(name) {
					return Err(crate::error::ComposeError::ServiceNotFound(name.clone()));
				}
			}
			target_services.to_vec()
		};

		for name in &names {
			let service = &file.services[name];
			if service.build.is_some() {
				self.build_service(name, service, file).await?;
			}
		}
		Ok(())
	}

	pub(super) async fn build_service(
		&self,
		service_name: &str,
		service: &Service,
		file: &crate::compose::types::ComposeFile,
	) -> Result<()> {
		let build = match &service.build {
			Some(b) => b,
			None => return Ok(()),
		};

		let context_path = self.base_dir.join(build.context());
		let tag = service
			.image
			.clone()
			.unwrap_or_else(|| format!("{}:latest", service_name));

		info!("building {tag} from {}", context_path.display());

		// Resolve `build.secrets` to in-tar files before building the context:
		// each secret value is shipped inside the build-context tar and referenced
		// by a relative `src=` path, which is the form the libpod build endpoint
		// expects (`env=`/host-path forms don't work reliably over the socket).
		let (secret_files, secret_specs) = self.resolve_build_secrets(build, file)?;

		let inline = build.dockerfile_inline().map(|s| s.to_string());
		let df = build.dockerfile().unwrap_or("Dockerfile").to_string();
		let ctx = context_path.clone();
		let (tar_bytes, dockerfile_name) =
			tokio::task::spawn_blocking(move || -> Result<(Vec<u8>, String)> {
				if let Some(inline_s) = inline {
					build_context_tar_with_inline(&ctx, &inline_s, &secret_files)
				} else {
					let bytes = build_context_tar(&ctx, &df, &secret_files)?;
					Ok((bytes, df))
				}
			})
			.await
			.map_err(|e| ComposeError::Build(e.to_string()))??;

		let arg_map = build.args().to_map();
		let mut build_args: std::collections::HashMap<String, String> =
			std::collections::HashMap::new();
		for (k, v) in arg_map {
			let value = v.unwrap_or_else(|| std::env::var(&k).unwrap_or_default());
			build_args.insert(k, value);
		}

		let mut labels: std::collections::HashMap<String, String> =
			std::collections::HashMap::new();
		if let BuildConfig::Config { labels: l, .. } = build {
			labels.extend(l.to_map());
		}

		let network = if let BuildConfig::Config {
			network: Some(n), ..
		} = build
		{
			Some(n.clone())
		} else {
			None
		};
		let platform = if let BuildConfig::Config { platforms, .. } = build {
			platforms.first().cloned()
		} else {
			None
		};
		let shmsize = build
			.shm_size()
			.and_then(size::parse_memory)
			.map(|s| s as i32);
		let extrahosts_str = build.extra_hosts().join(",");
		let extrahosts = if extrahosts_str.is_empty() {
			None
		} else {
			Some(extrahosts_str)
		};
		let cachefrom = if build.cache_from().is_empty() {
			None
		} else {
			Some(serde_json::to_string(build.cache_from()).unwrap_or_default())
		};
		let buildargs_json = if build_args.is_empty() {
			None
		} else {
			Some(serde_json::to_string(&build_args).unwrap_or_default())
		};
		let labels_json = if labels.is_empty() {
			None
		} else {
			Some(serde_json::to_string(&labels).unwrap_or_default())
		};

		let mut qs = format!(
			"t={}&rm=true&nocache={}",
			urlencoded(&tag),
			build.no_cache()
		);
		qs.push_str(&format!("&dockerfile={}", urlencoded(&dockerfile_name)));
		if build.pull() {
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
		if let Some(target) = build.target() {
			qs.push_str(&format!("&target={}", urlencoded(target)));
		}
		if !secret_specs.is_empty() {
			let json = serde_json::to_string(&secret_specs).unwrap_or_default();
			qs.push_str(&format!("&secrets={}", urlencoded(&json)));
		}
		if !build.cache_to().is_empty() {
			let json = serde_json::to_string(build.cache_to()).unwrap_or_default();
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

		let path = format!("{API_PREFIX}/build?{qs}");
		let body_bytes = Bytes::from(tar_bytes);
		let resp = self
			.client
			.post_bytes_stream(&path, body_bytes, "application/x-tar")
			.await
			.map_err(ComposeError::Podman)?;
		let mut stream = crate::libpod::parse_json_lines::<BuildOutput>(resp.into_body());

		while let Some(result) = stream.next().await {
			match result {
				Ok(output) => {
					if !output.stream.is_empty() {
						print!("{}", output.stream);
					}
					if let Some(err) = output.error_detail.and_then(|e| e.message) {
						return Err(ComposeError::Build(err));
					}
					if let Some(err) = output.error {
						if !err.is_empty() {
							return Err(ComposeError::Build(err));
						}
					}
				}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}

		self.apply_extra_tags(build, &tag).await;
		Ok(())
	}

	/// Resolve `build.secrets` into `(in-tar files, secret specs)`.
	///
	/// Each referenced top-level secret is read (from `file:`, inline `content:`,
	/// or `environment:`) and returned as a `(tar-entry-name, bytes)` pair plus a
	/// matching `id=NAME,src=ENTRY` spec for the build endpoint's `secrets` param.
	/// `external` secrets cannot be forwarded over the API and are warned + skipped.
	fn resolve_build_secrets(
		&self,
		build: &BuildConfig,
		file: &crate::compose::types::ComposeFile,
	) -> Result<ResolvedBuildSecrets> {
		let mut files = Vec::new();
		let mut specs = Vec::new();
		for name in build.secrets() {
			let Some(config) = file.secrets.get(name) else {
				return Err(ComposeError::Unsupported(format!(
					"build secret '{name}' is not defined in the top-level secrets section"
				)));
			};
			let bytes: Vec<u8> = if let Some(host_path) = &config.file {
				std::fs::read(self.base_dir.join(host_path)).map_err(ComposeError::Io)?
			} else if let Some(content) = &config.content {
				content.clone().into_bytes()
			} else if let Some(env_var) = &config.environment {
				std::env::var(env_var)
					.map_err(|_| {
						ComposeError::Unsupported(format!(
							"build secret '{name}' references env var '{env_var}' which is not set"
						))
					})?
					.into_bytes()
			} else if config.external == Some(true) {
				warn!("build secret '{name}' is external; cannot forward over the libpod build API — skipping");
				continue;
			} else {
				continue;
			};
			let entry = format!(".podup-build-secret-{name}");
			specs.push(format!("id={name},src={entry}"));
			files.push((entry, bytes));
		}
		Ok((files, specs))
	}

	/// Apply any `build.tags` aliases to the freshly built image.
	async fn apply_extra_tags(&self, build: &BuildConfig, tag: &str) {
		for extra_tag in build.tags() {
			let (repo, tag_str) = extra_tag
				.rsplit_once(':')
				.map(|(r, t)| (r.to_string(), t.to_string()))
				.unwrap_or_else(|| (extra_tag.clone(), "latest".to_string()));
			let encoded_tag = urlencoded(tag);
			let tag_path = format!(
				"{API_PREFIX}/images/{encoded_tag}/tag?repo={}&tag={}",
				urlencoded(&repo),
				urlencoded(&tag_str),
			);
			if let Err(e) = self.client.post_empty_ok(&tag_path).await {
				warn!("failed to apply extra tag {extra_tag}: {e}");
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::Engine;
	use crate::libpod::Client;

	fn engine(base: std::path::PathBuf) -> Engine {
		Engine::with_base_dir(Client::new("/nonexistent.sock"), "p".into(), base)
	}

	fn build_of(file: &crate::compose::types::ComposeFile) -> &crate::compose::types::BuildConfig {
		file.services["app"].build.as_ref().unwrap()
	}

	#[test]
	fn build_secret_from_file_shipped_in_tar() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("token.txt"), b"s3cr3t").unwrap();
		let yaml = "services:\n  app:\n    build:\n      context: .\n      secrets:\n        - tok\nsecrets:\n  tok:\n    file: token.txt\n";
		let file = crate::compose::parse_str(yaml).unwrap();
		let e = engine(dir.path().to_path_buf());
		let (files, specs) = e.resolve_build_secrets(build_of(&file), &file).unwrap();
		assert_eq!(
			specs,
			vec!["id=tok,src=.podup-build-secret-tok".to_string()]
		);
		assert_eq!(files.len(), 1);
		assert_eq!(files[0].0, ".podup-build-secret-tok");
		assert_eq!(files[0].1, b"s3cr3t");
	}

	#[test]
	fn build_secret_content_inlined() {
		let yaml = "services:\n  app:\n    build:\n      context: .\n      secrets:\n        - c\nsecrets:\n  c:\n    content: inline-value\n";
		let file = crate::compose::parse_str(yaml).unwrap();
		let e = engine(std::env::temp_dir());
		let (files, _) = e.resolve_build_secrets(build_of(&file), &file).unwrap();
		assert_eq!(files[0].1, b"inline-value");
	}

	#[test]
	fn build_secret_external_is_skipped() {
		let yaml = "services:\n  app:\n    build:\n      context: .\n      secrets:\n        - ext\nsecrets:\n  ext:\n    external: true\n";
		let file = crate::compose::parse_str(yaml).unwrap();
		let e = engine(std::env::temp_dir());
		let (files, specs) = e.resolve_build_secrets(build_of(&file), &file).unwrap();
		assert!(files.is_empty());
		assert!(specs.is_empty());
	}

	#[test]
	fn build_secret_undefined_errors() {
		let yaml =
			"services:\n  app:\n    build:\n      context: .\n      secrets:\n        - missing\n";
		let file = crate::compose::parse_str(yaml).unwrap();
		let e = engine(std::env::temp_dir());
		assert!(e.resolve_build_secrets(build_of(&file), &file).is_err());
	}
}
