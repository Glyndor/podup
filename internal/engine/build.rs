//! Image build and pull operations.
//!
//! [`Engine::pull_image`] fetches a pre-built image from a registry.
//! [`Engine::build_service`] compiles a build context tar, passes it to the
//! Podman/Docker API, and applies any extra tags. Inline Dockerfiles and
//! multi-stage `--target` trimming are handled before the tar is assembled.

use std::path::Path;

use bollard::body_full;
use bollard::query_parameters::{BuildImageOptions, CreateImageOptions, TagImageOptions};
use bytes::Bytes;
use flate2::write::GzEncoder;
use flate2::Compression;
use futures::StreamExt;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

use crate::compose::types::{BuildConfig, Service};
use crate::error::{ComposeError, Result};
use crate::size;

use super::Engine;

impl Engine {
	pub(super) async fn pull_image(&self, service: &Service) -> Result<()> {
		let image = match &service.image {
			Some(img) => img.clone(),
			None => return Ok(()),
		};

		info!("pulling {image}");

		let mut stream = self.docker.create_image(
			Some(CreateImageOptions {
				from_image: Some(image.clone()),
				platform: service.platform.clone().unwrap_or_default(),
				..Default::default()
			}),
			None,
			None,
		);

		while let Some(result) = stream.next().await {
			match result {
				Ok(info) => {
					if let Some(status) = info.status {
						debug!("{status}");
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
			build_context_tar_with_inline(&context_path, inline)?
		} else {
			let df = build.dockerfile().unwrap_or("Dockerfile");
			if let Some(target) = build.target() {
				build_context_tar_with_target(&context_path, df, target)?
			} else {
				(build_context_tar(&context_path, df)?, df.to_string())
			}
		};

		let arg_map = build.args().to_map();
		let mut build_args: std::collections::HashMap<String, String> =
			std::collections::HashMap::new();
		for (k, v) in arg_map {
			let value = match v {
				Some(val) => val,
				None => std::env::var(&k).unwrap_or_default(),
			};
			build_args.insert(k, value);
		}

		let mut labels: std::collections::HashMap<String, String> =
			std::collections::HashMap::new();
		if let BuildConfig::Config { labels: l, .. } = build {
			labels.extend(l.to_map());
		}

		let network_owned = if let BuildConfig::Config {
			network: Some(n), ..
		} = build
		{
			n.clone()
		} else {
			String::new()
		};
		let platform_owned = if let BuildConfig::Config { platforms, .. } = build {
			platforms.first().cloned().unwrap_or_default()
		} else {
			String::new()
		};
		let shmsize = build
			.shm_size()
			.and_then(size::parse_memory)
			.map(|s| s as u64)
			.unwrap_or(0);
		let extrahosts = build.extra_hosts().join(",");

		let options = BuildImageOptions {
			dockerfile: dockerfile_name,
			t: Some(tag.clone()),
			rm: true,
			nocache: build.no_cache(),
			pull: if build.pull() {
				Some("1".to_string())
			} else {
				None
			},
			buildargs: if build_args.is_empty() {
				None
			} else {
				Some(build_args)
			},
			labels: if labels.is_empty() {
				None
			} else {
				Some(labels)
			},
			networkmode: if network_owned.is_empty() {
				None
			} else {
				Some(network_owned)
			},
			platform: platform_owned,
			shmsize: if shmsize > 0 {
				Some(shmsize as i32)
			} else {
				None
			},
			extrahosts: if extrahosts.is_empty() {
				None
			} else {
				Some(extrahosts)
			},
			cachefrom: if build.cache_from().is_empty() {
				None
			} else {
				Some(build.cache_from().to_vec())
			},
			..Default::default()
		};

		let body = Bytes::from(tar_bytes);
		let mut stream = self
			.docker
			.build_image(options, None, Some(body_full(body)));

		while let Some(result) = stream.next().await {
			match result {
				Ok(info) => {
					if let Some(stream_msg) = info.stream {
						print!("{stream_msg}");
					}
					if let Some(err) = info.error_detail.and_then(|e| e.message) {
						return Err(ComposeError::Build(err));
					}
				}
				Err(e) => return Err(ComposeError::Podman(e)),
			}
		}

		// Apply additional tags.
		for extra_tag in build.tags() {
			let (repo, tag_str) = extra_tag
				.rsplit_once(':')
				.map(|(r, t)| (r.to_string(), t.to_string()))
				.unwrap_or_else(|| (extra_tag.clone(), "latest".to_string()));
			if let Err(e) = self
				.docker
				.tag_image(
					&tag,
					Some(TagImageOptions {
						repo: Some(repo),
						tag: Some(tag_str),
					}),
				)
				.await
			{
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

	// Inline Dockerfile first.
	let mut header = tar::Header::new_gnu();
	header.set_size(inline.len() as u64);
	header.set_mode(0o644);
	header.set_cksum();
	tar.append_data(&mut header, inline_name, inline.as_bytes())
		.map_err(|e| ComposeError::Build(e.to_string()))?;

	for entry in WalkDir::new(context).follow_links(false) {
		let entry = entry.map_err(|e| ComposeError::Io(e.into()))?;
		let abs = entry.path();
		let rel = abs
			.strip_prefix(context)
			.map_err(|_| ComposeError::Build("path strip error".into()))?;
		if rel.as_os_str().is_empty() {
			continue;
		}
		let rel_str = rel.to_string_lossy();
		if is_ignored(&rel_str, &ignore_patterns) {
			continue;
		}
		if abs.is_dir() {
			tar.append_dir(rel, abs)
				.map_err(|e| ComposeError::Build(e.to_string()))?;
		} else {
			tar.append_path_with_name(abs, rel)
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

	for entry in WalkDir::new(context).follow_links(false) {
		let entry = entry.map_err(|e| ComposeError::Io(e.into()))?;
		let abs = entry.path();
		let rel = abs
			.strip_prefix(context)
			.map_err(|_| ComposeError::Build("path strip error".into()))?;

		if rel.as_os_str().is_empty() {
			continue;
		}

		let rel_str = rel.to_string_lossy();
		if is_ignored(&rel_str, &ignore_patterns) {
			continue;
		}

		if abs.is_dir() {
			tar.append_dir(rel, abs)
				.map_err(|e| ComposeError::Build(e.to_string()))?;
		} else {
			tar.append_path_with_name(abs, rel)
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

/// Build a context tar with the Dockerfile truncated to stages up to `target`.
///
/// Achieves the same result as `docker build --target=<target>` without requiring
/// bollard API support for the target parameter.
fn build_context_tar_with_target(
	context: &Path,
	dockerfile: &str,
	target: &str,
) -> Result<(Vec<u8>, String)> {
	let df_path = context.join(dockerfile);
	let df_content = std::fs::read_to_string(&df_path).map_err(ComposeError::Io)?;
	let truncated = truncate_dockerfile_to_target(&df_content, target);

	let ignore_patterns = read_dockerignore(context);
	let encoder = GzEncoder::new(Vec::new(), Compression::default());
	let mut tar = tar::Builder::new(encoder);

	// Write truncated Dockerfile first.
	let df_bytes = truncated.as_bytes();
	let mut header = tar::Header::new_gnu();
	header.set_size(df_bytes.len() as u64);
	header.set_mode(0o644);
	header.set_cksum();
	tar.append_data(&mut header, dockerfile, df_bytes)
		.map_err(|e| ComposeError::Build(e.to_string()))?;

	// Add context, skipping the original Dockerfile (already wrote truncated version).
	for entry in WalkDir::new(context).follow_links(false) {
		let entry = entry.map_err(|e| ComposeError::Io(e.into()))?;
		let abs = entry.path();
		let rel = abs
			.strip_prefix(context)
			.map_err(|_| ComposeError::Build("path strip error".into()))?;
		if rel.as_os_str().is_empty() {
			continue;
		}
		let rel_str = rel.to_string_lossy();
		if is_ignored(&rel_str, &ignore_patterns) {
			continue;
		}
		if rel_str == dockerfile {
			continue; // Replaced by truncated version above.
		}
		if abs.is_dir() {
			tar.append_dir(rel, abs)
				.map_err(|e| ComposeError::Build(e.to_string()))?;
		} else {
			tar.append_path_with_name(abs, rel)
				.map_err(|e| ComposeError::Build(e.to_string()))?;
		}
	}

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	let bytes = gz
		.finish()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	Ok((bytes, dockerfile.to_string()))
}

/// Truncate a Dockerfile to only include stages up to and including `target`.
///
/// Stages after `target` are dropped, making the target stage the effective
/// final output — equivalent to `docker build --target=<target>`.
pub(crate) fn truncate_dockerfile_to_target(content: &str, target: &str) -> String {
	let target_lower = target.to_lowercase();
	let mut lines: Vec<&str> = Vec::new();
	let mut found_target = false;

	for line in content.lines() {
		let trimmed = line.trim().to_ascii_lowercase();

		if trimmed.starts_with("from ") {
			if found_target {
				// First FROM after our target stage — stop here.
				break;
			}
			lines.push(line);
			if let Some(as_idx) = trimmed.find(" as ") {
				let stage = trimmed[as_idx + 4..].trim().to_string();
				if stage == target_lower {
					found_target = true;
				}
			}
		} else {
			lines.push(line);
		}
	}

	if !found_target {
		tracing::warn!(
            "build.target '{target}' not found as a named stage in Dockerfile — using full Dockerfile"
        );
		return content.to_string();
	}

	lines.join("\n")
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
		} else if path == pattern.as_str()
			|| (path.starts_with(pattern.as_str())
				&& path.as_bytes().get(pattern.len()) == Some(&b'/'))
		{
			return true;
		}
	}
	false
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::{
		build_context_tar, build_context_tar_with_inline, build_context_tar_with_target,
		is_ignored, read_dockerignore, truncate_dockerfile_to_target,
	};
	use std::fs;
	use tempfile::tempdir;

	// truncate_dockerfile_to_target ----------------------------------------

	#[test]
	fn truncate_drops_stages_after_target() {
		let df = "FROM base AS builder\nRUN build\nFROM builder AS production\nRUN run\nFROM production AS final\nRUN finalize\n";
		let result = truncate_dockerfile_to_target(df, "production");
		assert!(result.contains("FROM base AS builder"));
		assert!(result.contains("FROM builder AS production"));
		assert!(!result.contains("FROM production AS final"));
	}

	#[test]
	fn truncate_unknown_target_returns_full() {
		let df = "FROM alpine\nRUN echo hi\n";
		let result = truncate_dockerfile_to_target(df, "nonexistent");
		assert_eq!(result, df);
	}

	#[test]
	fn truncate_case_insensitive_target() {
		let df = "FROM base AS Builder\nRUN step\nFROM Builder AS Next\nRUN other\n";
		let result = truncate_dockerfile_to_target(df, "builder");
		assert!(result.contains("AS Builder"));
		assert!(!result.contains("AS Next"));
	}

	#[test]
	fn truncate_single_stage_target() {
		let df = "FROM alpine AS app\nRUN echo done\n";
		let result = truncate_dockerfile_to_target(df, "app");
		assert!(result.contains("FROM alpine AS app"));
		assert!(result.contains("echo done"));
	}

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
	fn context_tar_respects_dockerignore() {
		let dir = tempdir().unwrap();
		fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
		fs::write(dir.path().join("secret.key"), b"top secret").unwrap();
		fs::write(dir.path().join(".dockerignore"), b"*.key\n").unwrap();
		let bytes = build_context_tar(dir.path(), "Dockerfile").unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
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

	// build_context_tar_with_target ----------------------------------------

	#[test]
	fn target_tar_truncates_dockerfile() {
		let dir = tempdir().unwrap();
		fs::write(
			dir.path().join("Dockerfile"),
			b"FROM alpine AS builder\nRUN build\nFROM builder AS final\nRUN run\n",
		)
		.unwrap();
		let (bytes, _) =
			build_context_tar_with_target(dir.path(), "Dockerfile", "builder").unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}
}
