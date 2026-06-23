//! Build context tar assembly and `.dockerignore` matching.
//!
//! Walks the build context directory, applies `.dockerignore` semantics
//! (last-match-wins with `!` re-includes, `*`/`?`/`**` globs), and packs the
//! result into a gzipped tar suitable for the libpod build endpoint.

use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::error::{ComposeError, Result};

/// Append the build context to `tar`, honoring `.dockerignore`.
fn append_context<W: std::io::Write>(
	tar: &mut tar::Builder<W>,
	context: &Path,
	ignore_patterns: &[String],
) -> Result<()> {
	for abs in super::super::walk_dir(context).map_err(ComposeError::Io)? {
		let rel = abs
			.strip_prefix(context)
			.map_err(|_| ComposeError::Build("path strip error".into()))?;
		let rel_str = rel.to_string_lossy();
		if is_ignored(&rel_str, ignore_patterns) {
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
	Ok(())
}

/// Append synthesized files (e.g. build secrets) to `tar` at the context root.
///
/// Build-secret bytes are placed in the build context as `.podup-build-secret-*`
/// entries so the libpod build endpoint can expose them through its BuildKit
/// `secrets=id=NAME,src=ENTRY` mount; they ride inside the context tar by design
/// of that mount mechanism and are not part of the user's source tree.
fn append_extra_files<W: std::io::Write>(
	tar: &mut tar::Builder<W>,
	extra_files: &[(String, Vec<u8>)],
) -> Result<()> {
	for (name, bytes) in extra_files {
		let mut header = tar::Header::new_gnu();
		header.set_size(bytes.len() as u64);
		header.set_mode(0o600);
		header.set_cksum();
		tar.append_data(&mut header, name, bytes.as_slice())
			.map_err(|e| ComposeError::Build(e.to_string()))?;
	}
	Ok(())
}

/// Write inline Dockerfile content into the context tar as `.dockerfile-inline`.
pub(super) fn build_context_tar_with_inline(
	context: &Path,
	inline: &str,
	extra_files: &[(String, Vec<u8>)],
) -> Result<(Vec<u8>, String)> {
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

	append_context(&mut tar, context, &ignore_patterns)?;
	append_extra_files(&mut tar, extra_files)?;

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	let bytes = gz
		.finish()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	Ok((bytes, inline_name.to_string()))
}

pub(crate) fn build_context_tar(
	context: &Path,
	_dockerfile: &str,
	extra_files: &[(String, Vec<u8>)],
) -> Result<Vec<u8>> {
	let ignore_patterns = read_dockerignore(context);

	let encoder = GzEncoder::new(Vec::new(), Compression::default());
	let mut tar = tar::Builder::new(encoder);

	append_context(&mut tar, context, &ignore_patterns)?;
	append_extra_files(&mut tar, extra_files)?;

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	let bytes = gz
		.finish()
		.map_err(|e| ComposeError::Build(e.to_string()))?;

	Ok(bytes)
}

/// Map a compose `additional_contexts` value to the libpod
/// `additionalbuildcontexts` form: `image:`, `url:`, or `localpath:`.
pub(super) fn map_additional_context(base_dir: &Path, value: &str) -> String {
	if let Some(img) = value.strip_prefix("docker-image://") {
		format!("image:{img}")
	} else if value.starts_with("http://")
		|| value.starts_with("https://")
		|| value.starts_with("git://")
	{
		format!("url:{value}")
	} else {
		format!("localpath:{}", base_dir.join(value).display())
	}
}

fn read_dockerignore(context: &Path) -> Vec<String> {
	let path = context.join(".dockerignore");
	let Ok(content) = crate::filesystem::read_to_string_capped(path) else {
		return Vec::new();
	};
	content
		.lines()
		.map(|l| l.trim().to_string())
		.filter(|l| !l.is_empty() && !l.starts_with('#'))
		.collect()
}

/// Decide whether `path` is excluded from the build context.
///
/// Patterns are evaluated in order and the **last** match wins, matching Docker
/// `.dockerignore` semantics: a leading `!` re-includes a path that an earlier
/// pattern excluded. So `*.log` then `!keep.log` ignores every log except
/// `keep.log`.
fn is_ignored(path: &str, patterns: &[String]) -> bool {
	let mut ignored = false;
	for pattern in patterns {
		let (negated, pat) = match pattern.strip_prefix('!') {
			Some(rest) => (true, rest),
			None => (false, pattern.as_str()),
		};
		if pattern_matches(pat, path) {
			ignored = !negated;
		}
	}
	ignored
}

/// Match a single (already de-negated) `.dockerignore` pattern against `path`.
fn pattern_matches(pattern: &str, path: &str) -> bool {
	if pattern.is_empty() {
		return false;
	}
	// Directory pattern (`foo/`): match the directory and everything beneath it.
	if let Some(dir) = pattern.strip_suffix('/') {
		return path == dir || path.starts_with(&format!("{dir}/"));
	}
	if pattern.contains('*') || pattern.contains('?') {
		return glob_match(pattern, path);
	}
	// Plain pattern: exact match, or a path segment prefix (`vendor` matches
	// `vendor/lib.rs`).
	path == pattern
		|| (path.starts_with(pattern) && path.as_bytes().get(pattern.len()) == Some(&b'/'))
}

/// Match path against a glob pattern.
///
/// Patterns without `/` are matched against the filename only, so `*.log`
/// excludes both `error.log` and `logs/error.log`. A single `*` never crosses a
/// `/` boundary; `**` matches any number of path segments (including `/`), so
/// `**/*.key` and `a/**/b` work like Docker.
fn glob_match(pattern: &str, path: &str) -> bool {
	if !pattern.contains('/') && !pattern.contains("**") {
		let filename = path.rsplit('/').next().unwrap_or(path);
		return glob_rec(pattern.as_bytes(), filename.as_bytes());
	}
	glob_rec(pattern.as_bytes(), path.as_bytes())
}

/// Backtracking glob matcher: `?` matches one non-`/` char, `*` matches any run
/// of non-`/` chars, `**` matches any run including `/`.
fn glob_rec(pat: &[u8], s: &[u8]) -> bool {
	if pat.is_empty() {
		return s.is_empty();
	}
	// `**` — matches across `/` boundaries.
	if pat.starts_with(b"**") {
		let mut rest = &pat[2..];
		// `**/` may also match zero directories, so `**/foo` matches top-level `foo`.
		if rest.first() == Some(&b'/') && glob_rec(&rest[1..], s) {
			return true;
		}
		if rest.is_empty() {
			rest = b"";
		}
		// Try consuming any prefix of `s` (including `/`).
		for i in 0..=s.len() {
			if glob_rec(rest, &s[i..]) {
				return true;
			}
		}
		return false;
	}
	match pat[0] {
		b'*' => {
			// Match any run of non-`/` chars.
			let mut i = 0;
			loop {
				if glob_rec(&pat[1..], &s[i..]) {
					return true;
				}
				if i >= s.len() || s[i] == b'/' {
					return false;
				}
				i += 1;
			}
		}
		b'?' => !s.is_empty() && s[0] != b'/' && glob_rec(&pat[1..], &s[1..]),
		c => !s.is_empty() && s[0] == c && glob_rec(&pat[1..], &s[1..]),
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::{
		build_context_tar, build_context_tar_with_inline, glob_match, is_ignored,
		map_additional_context, read_dockerignore,
	};
	use std::fs;
	use std::path::Path;
	use tempfile::tempdir;

	#[test]
	fn additional_context_prefix_mapping() {
		let base = Path::new("/proj");
		assert_eq!(
			map_additional_context(base, "docker-image://alpine"),
			"image:alpine"
		);
		assert_eq!(
			map_additional_context(base, "https://example.org/ctx.tar"),
			"url:https://example.org/ctx.tar"
		);
		assert_eq!(
			map_additional_context(base, "git://example.org/r.git"),
			"url:git://example.org/r.git"
		);
		// Local paths are resolved against base_dir using the host's native path
		// separator, so compare against a `join`ed path rather than a literal.
		let local = map_additional_context(base, "sub/dir");
		assert!(local.starts_with("localpath:"));
		assert!(local.ends_with(&base.join("sub/dir").display().to_string()));
	}

	#[test]
	fn extra_files_are_added_to_context_tar() {
		use flate2::read::GzDecoder;
		use std::io::Read;

		let dir = tempdir().unwrap();
		fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
		let extra = vec![(".podup-build-secret-tok".to_string(), b"hunter2".to_vec())];
		let bytes = build_context_tar(dir.path(), "Dockerfile", &extra).unwrap();

		let mut raw = Vec::new();
		GzDecoder::new(bytes.as_slice())
			.read_to_end(&mut raw)
			.unwrap();
		let mut archive = tar::Archive::new(raw.as_slice());
		let names: Vec<String> = archive
			.entries()
			.unwrap()
			.filter_map(|e| e.ok())
			.filter_map(|e| e.path().ok().map(|p| p.to_string_lossy().into_owned()))
			.collect();
		assert!(
			names.iter().any(|n| n.contains(".podup-build-secret-tok")),
			"secret entry must be present: {names:?}"
		);
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

	#[test]
	fn glob_match_double_star_any_depth() {
		assert!(glob_match("**/*.key", "secret.key"));
		assert!(glob_match("**/*.key", "a/b/c/secret.key"));
		assert!(glob_match("a/**/b", "a/b"));
		assert!(glob_match("a/**/b", "a/x/y/b"));
		assert!(!glob_match("a/**/b", "z/b"));
	}

	#[test]
	fn glob_match_question_mark() {
		assert!(glob_match("file?.txt", "file1.txt"));
		assert!(!glob_match("file?.txt", "file.txt"));
		assert!(!glob_match("file?.txt", "file12.txt"));
	}

	#[test]
	fn dockerignore_negation_reincludes() {
		let patterns = vec!["*.log".to_string(), "!keep.log".to_string()];
		assert!(is_ignored("error.log", &patterns));
		assert!(!is_ignored("keep.log", &patterns));
	}

	#[test]
	fn dockerignore_negation_order_matters() {
		// Re-include then exclude again: last match wins.
		let patterns = vec![
			"logs/".to_string(),
			"!logs/keep/".to_string(),
			"logs/keep/secret.txt".to_string(),
		];
		assert!(is_ignored("logs/a.log", &patterns));
		assert!(!is_ignored("logs/keep/b.log", &patterns));
		assert!(is_ignored("logs/keep/secret.txt", &patterns));
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
		let bytes = build_context_tar(dir.path(), "Dockerfile", &[]).unwrap();
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
		let bytes = build_context_tar(dir.path(), "Dockerfile", &[]).unwrap();

		// Decompress and scan for secret.key in tar entry names.
		let mut gz_content = Vec::new();
		GzDecoder::new(bytes.as_slice())
			.read_to_end(&mut gz_content)
			.unwrap();
		let mut archive = tar::Archive::new(gz_content.as_slice());
		let names: Vec<String> = archive
			.entries()
			.unwrap()
			.filter_map(|e| e.ok())
			.filter_map(|e| e.path().ok().map(|p| p.to_string_lossy().into_owned()))
			.collect();
		assert!(
			!names.iter().any(|n| n.contains("secret.key")),
			"secret.key must be excluded: {names:?}"
		);
	}

	#[test]
	fn context_tar_with_subdirectory() {
		let dir = tempdir().unwrap();
		fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
		fs::create_dir(dir.path().join("src")).unwrap();
		fs::write(dir.path().join("src/main.rs"), b"fn main() {}").unwrap();
		let bytes = build_context_tar(dir.path(), "Dockerfile", &[]).unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}

	// build_context_tar_with_inline ----------------------------------------

	#[test]
	fn inline_tar_produces_valid_gzip() {
		let dir = tempdir().unwrap();
		fs::write(dir.path().join("app.txt"), b"content").unwrap();
		let inline = "FROM alpine\nRUN echo hello\n";
		let (bytes, df_name) = build_context_tar_with_inline(dir.path(), inline, &[]).unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
		assert!(!df_name.is_empty());
	}
}
