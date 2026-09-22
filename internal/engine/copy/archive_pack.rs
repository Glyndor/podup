//! `cp`'s host->container packer.
//!
//! [`pack_path`] is the only `cp` packer: it walks the source and appends
//! every entry to a caller-supplied [`tar::Builder`], pushing each entry into
//! a caller-supplied `Vec<SentEntry>` at the same time. The streaming upload
//! path ([`super::pack::pack_path_stream`]) drives it with a writer that
//! hands bytes to a bounded channel; the archive tests drive it with a
//! `Vec<u8>` and feed the result to [`super::verify::sent_entries`] to pin
//! the read-back contract. One function, one walk, no parallel packers that
//! can drift apart while the tests stay green (#1844).
//!
//! The bytes the streaming writer carries are the bytes that go out as the
//! PUT body; the recorder is what the post-PUT confirmation reads when those
//! bytes are gone. Keeping the recorder next to the writer means the two
//! cannot disagree about what an entry is or what path it lives at: every
//! entry the tar gets is one the recorder sees, in the same order, with the
//! same kind. The parity test in [`super::pack_tests`] pins this by packing
//! to a `Vec<u8>` with the recorder on, then asserting the recorder equals
//! what `sent_entries` reads back from the same bytes.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{ComposeError, Result};

use super::pack_common::{record_one, walk, KindDispatch};
use super::verify::SentEntry;

/// Pack a host file or directory into a `tar::Builder`, recording every entry
/// appended.
///
/// `src`, `follow_link`, `name_override` and `contents` carry the same
/// meaning they always did for `cp`'s archive path:
///
/// - `src`: the file or directory to copy.
/// - `follow_link`: `-L`/`--follow-link`. Archive the symlink target's
///   contents instead of the link itself. Propagated into
///   [`super::pack_common::record_one`], which classifies with `metadata()`
///   when set and writes the target as a regular file or directory (with its
///   own descendants walked when the target is a directory).
/// - `name_override`: rename the top-level wrapper entry in non-`contents`
///   mode. The contents branch ignores this — every descendant lands at its
///   relative path.
/// - `contents`: drop the wrapper entry; pack every descendant at its
///   relative path inside `src`. `docker cp` semantics for a directory
///   source.
///
/// `tar` is the writer side: the streaming path passes a builder over a
/// [`super::pack::ChannelWriter`]; tests pass a builder over a `Vec<u8>`.
/// `sent` is the recorder: every entry appended is pushed here so the
/// post-PUT confirmation can compare the destination against what was
/// uploaded once the bytes are gone.
///
/// The recorded list matches what [`super::verify::sent_entries`] would
/// read back from the same archive: same path, same kind, same order. The
/// parity test in [`super::pack_tests`] asserts this on every shape the
/// existing tests cover.
pub(super) fn pack_path<W: Write>(
	src: &Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
) -> Result<()> {
	if contents {
		return pack_contents(src, follow_link, tar, sent);
	}

	let default: &Path = if src.is_dir() {
		Path::new(src.file_name().unwrap_or(OsStr::new(".")))
	} else {
		Path::new(src.file_name().unwrap_or(OsStr::new("file")))
	};
	let name: &Path = match name_override {
		Some(over) => Path::new(over),
		None => default,
	};

	if src.is_dir() {
		// Descendants first, then the wrapper. `tree_landed` walks the
		// recorded list last-to-first so a truncated upload (the tail is
		// gone) is refused on the first stat instead of the last; putting the
		// wrapper at the end preserves the ordering `append_dir_all` had.
		for abs in walk::walk_dir(src).map_err(ComposeError::Io)? {
			let rel = abs.strip_prefix(src).map_err(|_| {
				ComposeError::Build(format!(
					"cp: walk produced path outside source: {}",
					abs.display()
				))
			})?;
			let mut child_name = PathBuf::from(name);
			child_name.push(rel);
			let is_dir = abs.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
			if is_dir {
				record_one(
					tar,
					sent,
					&child_name,
					&abs,
					KindDispatch::Dir,
					follow_link,
					cp_err,
				)?;
			} else {
				record_one(
					tar,
					sent,
					&child_name,
					&abs,
					KindDispatch::FileOrLink,
					follow_link,
					cp_err,
				)?;
			}
		}
		// The wrapper entry: a directory named `name` whose path on disk is
		// `src`. `append_dir_all` would emit this same entry as the last
		// thing it did (DFS pushes children first, then the parent).
		record_one(tar, sent, name, src, KindDispatch::Dir, follow_link, cp_err)?;
	} else {
		record_one(
			tar,
			sent,
			name,
			src,
			KindDispatch::FileOrLink,
			follow_link,
			cp_err,
		)?;
	}
	Ok(())
}

/// `contents` branch: every descendant of `src` packed at its relative path,
/// with no wrapper entry. Mirrors the existing `pack_path` semantics.
fn pack_contents<W: Write>(
	src: &Path,
	follow_link: bool,
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
) -> Result<()> {
	if !src.is_dir() {
		return Err(ComposeError::Copy(format!(
			"cp: not a directory: {}",
			src.display()
		)));
	}
	for abs in walk::walk_dir(src).map_err(ComposeError::Io)? {
		let rel = abs.strip_prefix(src).map_err(|_| {
			ComposeError::Build(format!(
				"cp: walk produced path outside source: {}",
				abs.display()
			))
		})?;
		let is_dir = abs.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
		if is_dir {
			record_one(tar, sent, rel, &abs, KindDispatch::Dir, follow_link, cp_err)?;
		} else {
			record_one(
				tar,
				sent,
				rel,
				&abs,
				KindDispatch::FileOrLink,
				follow_link,
				cp_err,
			)?;
		}
	}
	Ok(())
}

/// Map an internal error to a `cp`-category `ComposeError`. Used by the
/// shared [`super::pack_common::record_one`] helper.
fn cp_err(msg: &str) -> ComposeError {
	ComposeError::Copy(format!("cp: {msg}"))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "archive_pack_tests.rs"]
mod tests;
