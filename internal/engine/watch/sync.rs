//! Sync-tar assembly and include/ignore filtering for watch rules.
//!
//! [`build_sync_tar`] is the unified watch-sync packer: it walks the changed
//! file or directory, appends each entry to a caller-supplied
//! [`tar::Builder`], and pushes each entry into a caller-supplied
//! `Vec<SentEntry>` at the same time. The streaming upload path
//! ([`crate::engine::copy::build_sync_tar_stream_for_watch`]) drives it with
//! a writer that hands bytes to a bounded channel; the archive tests drive it
//! with a `Vec<u8>` wrapped in a [`flate2::write::GzEncoder`]. One function,
//! one walk, no parallel packer that can drift while the tests stay green.
//!
//! The shared walk-and-record helpers live in
//! [`crate::engine::copy::pack_common`]; the difference between the `cp` and
//! the sync packers is the error category (`Copy` vs `Watch`), which the
//! caller passes to the shared helper as an error-mapping closure.

use std::io::Write;
use std::path::Path;

use crate::error::{ComposeError, Result};

use crate::engine::copy::pack_common::{record_one, walk, KindDispatch};
use crate::engine::copy::verify::SentEntry;

/// Pack `src` into a gzipped tar, storing its top-level entry under
/// `entry_name`.
///
/// `entry_name` is the archive path the changed file or directory should occupy
/// once extracted at the PUT destination. For a single changed file this is the
/// file's path relative to the watch-rule root (subdirectories preserved), or
/// the rename target's basename when the rule watches a single file. For a
/// directory `src`, every walked descendant is stored under `entry_name`,
/// preserving the in-tree layout.
///
/// `tar` is the writer side: the streaming path passes a builder over a gzip
/// encoder; the tests pass a builder over a `Vec<u8>` wrapped the same way.
/// `sent` is the recorder, parallel to the cp packer's recorder; the
/// post-PUT confirmation reads it back once the bytes are gone.
///
/// Watch sync stores symlinks as links: a symlink inside the watched tree
/// would otherwise copy the contents of its (possibly out-of-tree) target
/// into the container. The tar builder is told via
/// [`tar::Builder::follow_symlinks`] by the caller; this function does not
/// follow symlinks itself, so the `follow_link` parameter is hard-coded to
/// `false` at every call site.
pub(in crate::engine) fn build_sync_tar<W: Write>(
	src: &Path,
	entry_name: &Path,
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
) -> Result<()> {
	if src.is_dir() {
		for abs in walk::walk_dir(src).map_err(watch_io)? {
			let rel = abs
				.strip_prefix(src)
				.map_err(|_| watch_err(format!("path strip: {}", abs.display())))?;
			// Re-root each descendant under `entry_name` so the directory lands at
			// the rule target with its in-tree layout preserved.
			let name = entry_name.join(rel);
			// Classify without following symlinks so a symlink-to-dir is stored as
			// a link, not dereferenced.
			let is_dir = abs.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
			if is_dir {
				record_one(tar, sent, &name, &abs, KindDispatch::Dir, false, watch_tar)?;
			} else {
				record_one(
					tar,
					sent,
					&name,
					&abs,
					KindDispatch::FileOrLink,
					false,
					watch_tar,
				)?;
			}
		}
	} else {
		record_one(
			tar,
			sent,
			entry_name,
			src,
			KindDispatch::FileOrLink,
			false,
			watch_tar,
		)?;
	}

	Ok(())
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Classify a host-side IO error as a `watch` error. The watch dispatch
/// promises the user a `sync` failure will read as a sync failure, not a build
/// failure: `docs/commands.md` says the only signal a long-running `watch`
/// leaves open is the warning line, so a category swap from `sync` to `build`
/// silently drops the original context. `build` is reserved for image build.
fn watch_io(e: std::io::Error) -> ComposeError {
	watch_err(e.to_string())
}

/// Classify a tar-pack error as a `watch` error. Same reasoning as
/// [`watch_io`]: the failure is in the watch sync path, and the warning line
/// must keep that category.
fn watch_tar(msg: &str) -> ComposeError {
	watch_err(msg.to_string())
}

fn watch_err(msg: String) -> ComposeError {
	ComposeError::Watch(format!("sync: {msg}"))
}

/// True when `path` matches a watch-rule `ignore` pattern. A pattern ending in
/// `/` matches `path` by directory prefix; otherwise it matches an exact path or
/// a leading path segment (the pattern followed by `/`). Matching is anchored at
/// the start of `path`.
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

/// True when `path` matches a watch-rule `include` pattern. Unlike
/// [`is_ignored`], a `*.ext` pattern matches by extension suffix, and a bare name
/// matches not only an exact path or directory prefix but also a trailing path
/// segment anywhere in `path` (the pattern preceded by `/`).
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
#[path = "sync_tests.rs"]
mod tests;
