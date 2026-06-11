//! Image build and pull operations.
//!
//! [`Engine::pull_image`] fetches a pre-built image from a registry.
//! [`Engine::build_service`] compiles a build context tar, passes it to the
//! Podman libpod API, and applies any extra tags. Multi-stage targets are
//! passed as the `target=` query parameter — the full Dockerfile is always sent.

use std::path::Path;

use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::StreamExt;
use tracing::{debug, info, warn};

use crate::compose::types::{BuildConfig, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::{BuildOutput, ImagePullProgress};
use crate::libpod::urlencoded;
use crate::size;

use super::Engine;

impl Engine {
	pub(super) async fn pull_image(&self, service: &Service) -> Result<()> {
		let image = match &service.image {
			Some(img) => img.clone(),
			None => return Ok(()),
		};

		info!("pulling {image}");

		let mut query = format!("reference={}&policy=missing", urlencoded(&image));
		if let Some(platform) = &service.platform {
			query.push_str(&format!("&platform={}", urlencoded(platform)));
		}

		let path = format!("/libpod/images/pull?{query}");
		let resp = self.client.post_empty_stream(&path).await.map_err(ComposeError::Podman)?;
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
				self.build_service(name, service).await?;
			}
		}
		Ok(())
	}

	pub(super) async fn build_service(&self, service_name: &str, service: &Service) -> Result<()> {
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

		let (tar_bytes, dockerfile_name) = if let Some(inline) = build.dockerfile_inline() {
			let ctx = context_path.clone();
			let inline_s = inline.to_string();
			tokio::task::spawn_blocking(move || build_context_tar_with_inline(&ctx, &inline_s))
				.await
				.map_err(|e| ComposeError::Build(e.to_string()))??
		} else {
			let df = build.dockerfile().unwrap_or("Dockerfile");
			let ctx = context_path.clone();
			let df_s = df.to_string();
			let bytes = tokio::task::spawn_blocking(move || build_context_tar(&ctx, &df_s))
				.await
				.map_err(|e| ComposeError::Build(e.to_string()))??;
			(bytes, df.to_string())
		};

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

		let network = if let BuildConfig::Config { network: Some(n), .. } = build {
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
		let extrahosts = if extrahosts_str.is_empty() { None } else { Some(extrahosts_str) };
		let cachefrom = if build.cache_from().is_empty() {
			None
		} else {
			Some(
				serde_json::to_string(build.cache_from())
					.unwrap_or_default(),
			)
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

		let mut qs = format!("t={}&rm=true&nocache={}", urlencoded(&tag), build.no_cache());
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

		let path = format!("/libpod/build?{qs}");
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

		for extra_tag in build.tags() {
			let (repo, tag_str) = extra_tag
				.rsplit_once(':')
				.map(|(r, t)| (r.to_string(), t.to_string()))
				.unwrap_or_else(|| (extra_tag.clone(), "latest".to_string()));
			let encoded_tag = urlencoded(&tag);
			let tag_path = format!(
				"/libpod/images/{encoded_tag}/tag?repo={}&tag={}",
				urlencoded(&repo),
				urlencoded(&tag_str),
			);
			if let Err(e) = self.client.post_empty_ok(&tag_path).await {
				warn!("failed to apply extra tag {extra_tag}: {e}");
			}
		}

		Ok(())
	}
}

// ---------------------------------------------------------------------------
// Build context tar
// ---------------------------------------------------------------------------

/// Write inline Dockerfile content into the context tar as `.dockerfile-inline`.
fn build_context_tar_with_inline(context: &Path, inline: &str) -> Result<(Vec<u8>, String)> {
	let inline_name = ".dockerfile-inline";
	let ignore_patterns = read_dockerignore(context);

	let encoder = GzEncoder::new(Vec::new(), Compression::default());
	let mut tar = tar::Builder::new(encoder);

	let mut header = tar::Header::new_gnu();
	header.set_size(inline.len() as u64);
	header.set_mode(0o644);
	header.set_cksum();
	tar.append_data(&mut header, inline_name, inline.as_bytes())
		.map_err(|e| ComposeError::Build(e.to_string()))?;

	for abs in super::walk_dir(context).map_err(ComposeError::Io)? {
		let rel = abs
			.strip_prefix(context)
			.map_err(|_| ComposeError::Build("path strip error".into()))?;
		let rel_str = rel.to_string_lossy();
		if is_ignored(&rel_str, &ignore_patterns) {
			continue;
		}
		if abs.is_dir() {
			tar.append_dir(rel, &abs)
				.map_err(|e| ComposeError::Build(e.to_string()))?;
		} else {
			tar.append_path_with_name(&abs, rel)
				.map_err(|e| ComposeError::Build(e.to_string()))?;
		}
	}

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	let bytes = gz
		.finish()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	Ok((bytes, inline_name.to_string()))
}

pub(crate) fn build_context_tar(context: &Path, _dockerfile: &str) -> Result<Vec<u8>> {
	let ignore_patterns = read_dockerignore(context);

	let encoder = GzEncoder::new(Vec::new(), Compression::default());
	let mut tar = tar::Builder::new(encoder);

	for abs in super::walk_dir(context).map_err(ComposeError::Io)? {
		let rel = abs
			.strip_prefix(context)
			.map_err(|_| ComposeError::Build("path strip error".into()))?;
		let rel_str = rel.to_string_lossy();
		if is_ignored(&rel_str, &ignore_patterns) {
			continue;
		}
		if abs.is_dir() {
			tar.append_dir(rel, &abs)
				.map_err(|e| ComposeError::Build(e.to_string()))?;
		} else {
			tar.append_path_with_name(&abs, rel)
				.map_err(|e| ComposeError::Build(e.to_string()))?;
		}
	}

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	let bytes = gz
		.finish()
		.map_err(|e| ComposeError::Build(e.to_string()))?;

	Ok(bytes)
}

fn read_dockerignore(context: &Path) -> Vec<String> {
	let path = context.join(".dockerignore");
	let Ok(content) = std::fs::read_to_string(path) else {
		return Vec::new();
	};
	content
		.lines()
		.map(|l| l.trim().to_string())
		.filter(|l| !l.is_empty() && !l.starts_with('#'))
		.collect()
}

fn is_ignored(path: &str, patterns: &[String]) -> bool {
	for pattern in patterns {
		if pattern.ends_with('/') {
			if path.starts_with(pattern.as_str()) {
				return true;
			}
		} else if pattern.contains('*') {
			if glob_match(pattern, path) {
				return true;
			}
		} else if path == pattern.as_str()
			|| (path.starts_with(pattern.as_str())
				&& path.as_bytes().get(pattern.len()) == Some(&b'/'))
		{
			return true;
		}
	}
	false
}

/// Match path against a glob pattern.
///
/// Patterns without `/` are matched against the filename only, so `*.log`
/// excludes both `error.log` and `logs/error.log`. `*` never crosses a `/`
/// boundary when the pattern itself contains a `/`.
fn glob_match(pattern: &str, path: &str) -> bool {
	if !pattern.contains('/') {
		let filename = path.rsplit('/').next().unwrap_or(path);
		return match_star(pattern, filename);
	}
	match_star(pattern, path)
}

/// Match `s` against `pat` where `*` matches any sequence of non-`/` chars.
fn match_star(pat: &str, s: &str) -> bool {
	match pat.split_once('*') {
		None => s == pat,
		Some((prefix, rest)) => {
			s.starts_with(prefix) && {
				let s = &s[prefix.len()..];
				(0..=s.len())
					.take_while(|&i| !s[..i].contains('/'))
					.any(|i| match_star(rest, &s[i..]))
			}
		}
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::{
		build_context_tar, build_context_tar_with_inline, glob_match, is_ignored,
		read_dockerignore,
	};
	use std::fs;
	use tempfile::tempdir;

	// is_ignored (build) ---------------------------------------------------

	#[test]
	fn build_ignored_exact() {
		let patterns = vec!["secret.txt".to_string()];
		assert!(is_ignored("secret.txt", &patterns));
		assert!(!is_ignored("secret.txt.bak", &patterns));
	}

	#[test]
	fn build_ignored_dir() {
		let patterns = vec!["node_modules/".to_string()];
		assert!(is_ignored("node_modules/foo.js", &patterns));
		assert!(!is_ignored("other/foo.js", &patterns));
	}

	#[test]
	fn build_ignored_path_separator() {
		let patterns = vec!["vendor".to_string()];
		assert!(is_ignored("vendor/lib.rs", &patterns));
		assert!(!is_ignored("notvendor/lib.rs", &patterns));
	}

	#[test]
	fn build_ignored_glob_extension() {
		let patterns = vec!["*.key".to_string()];
		assert!(is_ignored("secret.key", &patterns));
		assert!(is_ignored("certs/ca.key", &patterns));
		assert!(!is_ignored("key.txt", &patterns));
	}

	#[test]
	fn build_ignored_glob_in_subdir() {
		let patterns = vec!["logs/*.log".to_string()];
		assert!(is_ignored("logs/error.log", &patterns));
		assert!(!is_ignored("other/error.log", &patterns));
	}

	#[test]
	fn glob_match_star_extension() {
		assert!(glob_match("*.env", "production.env"));
		assert!(glob_match("*.env", "config/.env"));
		assert!(!glob_match("*.env", "env.txt"));
	}

	#[test]
	fn glob_match_star_prefix() {
		assert!(glob_match("id_*", "id_rsa"));
		assert!(glob_match("id_*", "id_ed25519"));
		assert!(!glob_match("id_*", "not_id_rsa"));
	}

	// read_dockerignore ----------------------------------------------------

	#[test]
	fn dockerignore_parsed_correctly() {
		let dir = tempdir().unwrap();
		fs::write(
			dir.path().join(".dockerignore"),
			b"# comment\n\ntarget/\n*.log\n",
		)
		.unwrap();
		let patterns = read_dockerignore(dir.path());
		assert_eq!(patterns, vec!["target/", "*.log"]);
	}

	#[test]
	fn dockerignore_missing_returns_empty() {
		let dir = tempdir().unwrap();
		let patterns = read_dockerignore(dir.path());
		assert!(patterns.is_empty());
	}

	// build_context_tar ----------------------------------------------------

	#[test]
	fn context_tar_produces_valid_gzip() {
		let dir = tempdir().unwrap();
		fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
		fs::write(dir.path().join("app.rs"), b"fn main() {}").unwrap();
		let bytes = build_context_tar(dir.path(), "Dockerfile").unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}

	#[test]
	fn context_tar_excludes_dockerignore_glob() {
		use flate2::read::GzDecoder;
		use std::io::Read;

		let dir = tempdir().unwrap();
		fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
		fs::write(dir.path().join("secret.key"), b"top secret").unwrap();
		fs::write(dir.path().join(".dockerignore"), b"*.key\n").unwrap();
		let bytes = build_context_tar(dir.path(), "Dockerfile").unwrap();

		// Decompress and scan for secret.key in tar entry names.
		let mut gz_content = Vec::new();
		GzDecoder::new(bytes.as_slice()).read_to_end(&mut gz_content).unwrap();
		let mut archive = tar::Archive::new(gz_content.as_slice());
		let names: Vec<String> = archive
			.entries()
			.unwrap()
			.filter_map(|e| e.ok())
			.filter_map(|e| e.path().ok().map(|p| p.to_string_lossy().into_owned()))
			.collect();
		assert!(!names.iter().any(|n| n.contains("secret.key")), "secret.key must be excluded: {names:?}");
	}

	#[test]
	fn context_tar_with_subdirectory() {
		let dir = tempdir().unwrap();
		fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
		fs::create_dir(dir.path().join("src")).unwrap();
		fs::write(dir.path().join("src/main.rs"), b"fn main() {}").unwrap();
		let bytes = build_context_tar(dir.path(), "Dockerfile").unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}

	// build_context_tar_with_inline ----------------------------------------

	#[test]
	fn inline_tar_produces_valid_gzip() {
		let dir = tempdir().unwrap();
		fs::write(dir.path().join("app.txt"), b"content").unwrap();
		let inline = "FROM alpine\nRUN echo hello\n";
		let (bytes, df_name) = build_context_tar_with_inline(dir.path(), inline).unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
		assert!(!df_name.is_empty());
	}
}
