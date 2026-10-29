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

use crate::engine::context::walk::walk_dir;
use crate::engine::copy::pack_common::{append_pinned, record_one, walk, KindDispatch};
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
/// `skip` is consulted for every entry that would otherwise be recorded
/// (the root entry and each descendant of a directory), with the entry
/// name the packer would record it under. When `skip` returns `true` the
/// entry is dropped: a directory that is skipped is still walked, so a
/// safe deeper mount that sits below a skipped directory still receives
/// its subtree. The archive tests and any caller that wants today's
/// behaviour pass `&|_| false`.
///
/// Watch sync stores symlinks as links: a symlink inside the watched tree
/// would otherwise copy the contents of its (possibly out-of-tree) target
/// into the container. The tar builder is told via
/// [`tar::Builder::follow_symlinks`] by the caller; this function does not
/// follow symlinks itself, so the `follow_link` parameter is hard-coded to
/// `false` at every call site.
pub(in crate::backends::engine) fn build_sync_tar<W: Write>(
	src: &Path,
	entry_name: &Path,
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
	skip: &dyn Fn(&Path) -> bool,
) -> Result<()> {
	// `symlink_metadata` does not follow symlinks: a watched path that was
	// replaced by a symlink to a directory outside the rule keeps its link
	// shape and is stored as a link, not dereferenced into the link's target.
	// `is_dir` would follow the link and read the target's directory layout,
	// turning the upload into the contents of an out-of-tree directory
	// (#1985).
	let src_is_dir = src.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
	if src_is_dir {
		// Directory rule: when `entry_name` is not empty, the archive carries
		// the directory itself (so a file replaced by an empty directory is
		// reflected as a directory in the container, #1985) followed by every
		// descendant re-rooted under `entry_name`. An empty `entry_name` means
		// "the rule's own root"; the initial sync puts the descendants at the
		// target without a wrapper entry.
		if !entry_name.as_os_str().is_empty() && !skip(entry_name) {
			record_one(
				tar,
				sent,
				entry_name,
				src,
				KindDispatch::Dir,
				false,
				watch_tar,
			)?;
		}
		// Walked through descriptors, so the bytes of an entry are those of the
		// file the walk opened, not of whatever its name points at by the time
		// it is read. A directory the filter drops is still walked: a safe
		// deeper mount may sit below it.
		let mut failure: Option<ComposeError> = None;
		let walked = walk::visit_dir_skipping(
			src,
			|_| false,
			|_| true,
			|abs, entry| {
				let rel = abs
					.strip_prefix(src)
					.map_err(|_| std::io::Error::other("path strip"))?;
				// Re-root each descendant under `entry_name` so the directory lands
				// at the rule target with its in-tree layout preserved.
				if skip(&entry_name.join(rel)) {
					return Ok(());
				}
				append_pinned(tar, sent, src, Some(entry_name), abs, entry, &watch_tar).map_err(
					|e| {
						failure = Some(e);
						std::io::Error::other("sync: the entry could not be packed")
					},
				)
			},
		);
		if let Some(e) = failure {
			return Err(e);
		}
		walked.map_err(watch_io)?;
	} else if !skip(entry_name) {
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

/// True when the upload for `src` under `entry_name` would record at least
/// one entry that the per-entry `skip` filter does not drop. Used by the
/// watch sync to short-circuit an upload whose every entry maps back into
/// the watched tree: a tar that the filter stripped of every record would
/// still trigger the `mkdir -p` on the destination (creating a directory
/// through the bind) and would fail its verification step, the failure
/// would stop the restart or exec of a `sync+restart` / `sync+exec`
/// action, and `sync_all` would report the rule as failed.
///
/// The rules mirror [`build_sync_tar`]:
/// - `src` is a directory: the root entry is permitted when the filter
///   accepts `entry_name` (an empty `entry_name` checks the destination
///   directory itself), plus every walked descendant re-rooted under
///   `entry_name` whose joined name the filter accepts. A directory the
///   filter drops is still walked, so a safe deeper mount below a
///   loop-causing directory is still recorded.
/// - `src` is a single file: the file is permitted when the filter
///   accepts `entry_name`.
///
/// `src_is_dir` is read with `symlink_metadata` for the same reason
/// `build_sync_tar` uses it: a symlink that points at a directory
/// outside the rule must be uploaded as a link, not as the contents of
/// its target (#1985).
pub(in crate::backends::engine) fn has_permitted_entry(
	src: &Path,
	entry_name: &Path,
	skip: &dyn Fn(&Path) -> bool,
) -> std::io::Result<bool> {
	let src_is_dir = src.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
	if src_is_dir {
		if !skip(entry_name) {
			return Ok(true);
		}
		for abs in walk_dir(src)? {
			let rel = abs.strip_prefix(src).map_err(|e| {
				std::io::Error::other(format!("path strip: {}: {e}", abs.display()))
			})?;
			let name = entry_name.join(rel);
			if !skip(&name) {
				return Ok(true);
			}
		}
	} else if !skip(entry_name) {
		return Ok(true);
	}
	Ok(false)
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

/// Legacy project-relative `ignore` matcher.
///
/// Kept for compatibility with rules written before the spec-driven matcher:
/// when a pattern was not matched by the new rule-relative matcher (no pattern
/// matched at all) the old behaviour runs as a fallback, and a warning is
/// logged once per distinct pattern. Patterns are read against the
/// project-relative path, which is what watch did before the
/// `.dockerignore`-style semantics were adopted. A pattern ending in `/`
/// matches `path` by directory prefix; otherwise it matches an exact path or a
/// leading path segment (the pattern followed by `/`). Matching is anchored at
/// the start of `path`.
pub(in crate::backends::engine) fn legacy_project_relative_ignored(
	path: &str,
	patterns: &[String],
) -> bool {
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

/// Legacy project-relative `include` matcher.
///
/// Kept for compatibility with rules written before the spec-driven matcher:
/// when a pattern was not matched by the new rule-relative matcher the old
/// behaviour runs as a fallback. Unlike the ignore case, a `*.ext` pattern
/// matches by extension suffix, and a bare name matches not only an exact path
/// or directory prefix but also a trailing path segment anywhere in `path`.
pub(in crate::backends::engine) fn legacy_project_relative_included(
	path: &str,
	patterns: &[String],
) -> bool {
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
mod tests;

#[cfg(test)]
#[path = "sync_filter_tests.rs"]
mod filter_tests;
