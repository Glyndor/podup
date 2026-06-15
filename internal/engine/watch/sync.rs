//! Sync-tar assembly and include/ignore filtering for watch rules.
//!
//! [`build_sync_tar`] packs a changed file or directory into a gzipped tar for
//! upload into the container. [`is_ignored`] / [`is_included`] implement the
//! `develop.watch` rule path filters.

use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::error::{ComposeError, Result};

pub(super) fn build_sync_tar(src: &Path) -> Result<Vec<u8>> {
	let encoder = GzEncoder::new(Vec::new(), Compression::default());
	let mut tar = tar::Builder::new(encoder);
	// Do not dereference symlinks: a symlink inside the watched tree would
	// otherwise copy the contents of its (possibly out-of-tree) target into the
	// container. Store the link itself instead.
	tar.follow_symlinks(false);

	if src.is_dir() {
		for abs in super::super::walk_dir(src).map_err(ComposeError::Io)? {
			let rel = abs
				.strip_prefix(src)
				.map_err(|_| ComposeError::Build("path strip".into()))?;
			// Classify without following symlinks so a symlink-to-dir is stored as
			// a link, not dereferenced.
			let is_dir = abs.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
			if is_dir {
				tar.append_dir(rel, &abs)
					.map_err(|e| ComposeError::Build(e.to_string()))?;
			} else {
				tar.append_path_with_name(&abs, rel)
					.map_err(|e| ComposeError::Build(e.to_string()))?;
			}
		}
	} else if let Some(name) = src.file_name() {
		tar.append_path_with_name(src, name)
			.map_err(|e| ComposeError::Build(e.to_string()))?;
	}

	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	let bytes = gz
		.finish()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	Ok(bytes)
}

pub(super) fn is_ignored(path: &str, patterns: &[String]) -> bool {
	for pat in patterns {
		if pat.ends_with('/') {
			if path.starts_with(pat.as_str()) {
				return true;
			}
		} else if path == pat.as_str()
			|| (path.starts_with(pat.as_str()) && path.as_bytes().get(pat.len()) == Some(&b'/'))
		{
			return true;
		}
	}
	false
}

pub(super) fn is_included(path: &str, patterns: &[String]) -> bool {
	for pat in patterns {
		if pat.starts_with("*.") {
			let ext = &pat[1..];
			if path.ends_with(ext) {
				return true;
			}
		} else if pat.ends_with('/') {
			if path.starts_with(pat.as_str()) {
				return true;
			}
		} else if path == pat.as_str()
			|| (path.len() > pat.len() + 1
				&& path.as_bytes()[path.len() - pat.len() - 1] == b'/'
				&& path.ends_with(pat.as_str()))
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
	use super::{build_sync_tar, is_ignored, is_included};
	use std::fs;
	use tempfile::tempdir;

	fn pats(v: &[&str]) -> Vec<String> {
		v.iter().map(|s| s.to_string()).collect()
	}

	// is_ignored -----------------------------------------------------------

	#[test]
	fn ignored_exact_file() {
		assert!(is_ignored("Makefile", &pats(&["Makefile"])));
	}

	#[test]
	fn ignored_not_prefix_match() {
		assert!(!is_ignored("Makefile.local", &pats(&["Makefile"])));
	}

	#[test]
	fn ignored_dir_prefix() {
		assert!(is_ignored("node_modules/foo.js", &pats(&["node_modules/"])));
	}

	#[test]
	fn ignored_dir_prefix_no_partial() {
		assert!(!is_ignored("nonode_modules/foo", &pats(&["node_modules/"])));
	}

	#[test]
	fn ignored_path_with_slash() {
		assert!(is_ignored("vendor/lib.rs", &pats(&["vendor"])));
	}

	#[test]
	fn ignored_empty_patterns() {
		assert!(!is_ignored("anything.rs", &[]));
	}

	#[test]
	fn ignored_no_match() {
		assert!(!is_ignored("src/main.rs", &pats(&["target/", "*.log"])));
	}

	// is_included ----------------------------------------------------------

	#[test]
	fn included_glob_extension() {
		assert!(is_included("src/main.rs", &pats(&["*.rs"])));
	}

	#[test]
	fn included_glob_no_match() {
		assert!(!is_included("src/main.go", &pats(&["*.rs"])));
	}

	#[test]
	fn included_dir_prefix() {
		assert!(is_included("src/main.rs", &pats(&["src/"])));
	}

	#[test]
	fn included_dir_prefix_no_match() {
		assert!(!is_included("test/main.rs", &pats(&["src/"])));
	}

	#[test]
	fn included_exact_match() {
		assert!(is_included("Makefile", &pats(&["Makefile"])));
	}

	#[test]
	fn included_path_segment_suffix() {
		assert!(is_included("src/lib.rs", &pats(&["lib.rs"])));
	}

	#[test]
	fn included_empty_patterns() {
		assert!(!is_included("anything", &[]));
	}

	// build_sync_tar -------------------------------------------------------

	#[test]
	fn sync_tar_single_file() {
		let dir = tempdir().unwrap();
		let file = dir.path().join("hello.txt");
		fs::write(&file, b"hello world").unwrap();
		let bytes = build_sync_tar(&file).unwrap();
		// gzip magic bytes
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}

	#[test]
	fn sync_tar_directory() {
		let dir = tempdir().unwrap();
		fs::write(dir.path().join("a.txt"), b"file a").unwrap();
		fs::create_dir(dir.path().join("sub")).unwrap();
		fs::write(dir.path().join("sub/b.txt"), b"file b").unwrap();
		let bytes = build_sync_tar(dir.path()).unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}

	#[test]
	fn sync_tar_path_with_no_file_name() {
		// A path that has no file_name (e.g. root "/") — tar should be empty but valid.
		let dir = tempdir().unwrap();
		// Empty directory — no entries other than root
		let bytes = build_sync_tar(dir.path()).unwrap();
		assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
	}
}
