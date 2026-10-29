//! `cp`'s host->container packer.
//!
//! [`pack_path`] is the only `cp` packer: it walks the source and appends
//! every entry to a caller-supplied [`tar::Builder`], pushing each entry into
//! a caller-supplied `Vec<SentEntry>` at the same time. The streaming upload
//! path ([`super::transport::pack_path_stream`]) drives it with a writer that
//! hands bytes to a bounded channel; the archive tests drive it with a
//! `Vec<u8>` and feed the result to [`super::super::upload::verify::sent_entries`] to pin
//! the read-back contract. One function, one walk, no parallel packers that
//! can drift apart while the tests stay green (#1844).
//!
//! The bytes the streaming writer carries are the bytes that go out as the
//! PUT body; the recorder is what the post-PUT confirmation reads when those
//! bytes are gone. Keeping the recorder next to the writer means the two
//! cannot disagree about what an entry is or what path it lives at: every
//! entry the tar gets is one the recorder sees, in the same order, with the
//! same kind. The parity test in [`super::transport::tests`] pins this by packing
//! to a `Vec<u8>` with the recorder on, then asserting the recorder equals
//! what `sent_entries` reads back from the same bytes.
//!
//! A tree is packed through descriptors. The walk hands over each entry with
//! the descriptor it was opened with, and the tar is written from that
//! descriptor: a name that is replaced between the walk and the write cannot
//! redirect the bytes that go out. The root the user named is the one path
//! read by path, because that is the path they asked for.

use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{ComposeError, Result};

use super::super::upload::verify::{SentEntry, SentKind};
use super::common::{push_entry, record_one, walk, KindDispatch};

/// Pack a host file or directory into a `tar::Builder`, recording every entry
/// appended.
///
/// `src`, `follow_link`, `name_override` and `contents` carry the same
/// meaning they always did for `cp`'s archive path:
///
/// - `src`: the file or directory to copy.
/// - `follow_link`: `-L`/`--follow-link`. Archive the symlink target's
///   contents instead of the link itself. Propagated into
///   [`super::common::record_one`], which classifies with `metadata()`
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
/// [`super::transport::ChannelWriter`]; tests pass a builder over a `Vec<u8>`.
/// `sent` is the recorder: every entry appended is pushed here so the
/// post-PUT confirmation can compare the destination against what was
/// uploaded once the bytes are gone.
///
/// The recorded list matches what [`super::super::upload::verify::sent_entries`] would
/// read back from the same archive: same path, same kind, same order. The
/// parity test in [`super::transport::tests`] asserts this on every shape the
/// existing tests cover.
pub(in crate::backends::engine::host_files::copy) fn pack_path<W: Write>(
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
		if follow_link {
			pack_following_links(src, name, tar, sent)?;
			// The wrapper entry: a directory named `name` whose path on disk is
			// `src`. `append_dir_all` would emit this same entry as the last
			// thing it did (DFS pushes children first, then the parent).
			record_one(tar, sent, name, src, KindDispatch::Dir, follow_link, cp_err)?;
		} else {
			pack_pinned_tree(src, Some(name), tar, sent)?;
		}
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
	if follow_link {
		pack_following_links(src, Path::new(""), tar, sent)?;
	} else {
		pack_pinned_tree(src, None, tar, sent)?;
	}
	Ok(())
}

/// Pack a tree the way `-L` asks for: the walk names the entries and every
/// link is resolved to what it points at.
///
/// Resolving links is the policy `-L` asks for, so this path reads the tree
/// by name. The walk it uses is the one that follows links, and every entry
/// it names is handed to [`record_one`], which classifies it against the
/// path.
fn pack_following_links<W: Write>(
	src: &Path,
	prefix: &Path,
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
) -> Result<()> {
	for abs in walk::walk_dir(src).map_err(ComposeError::Io)? {
		let rel = abs.strip_prefix(src).map_err(|_| {
			ComposeError::Build(format!(
				"cp: walk produced path outside source: {}",
				abs.display()
			))
		})?;
		let child_name = prefix.join(rel);
		let is_dir = abs.symlink_metadata().map(|m| m.is_dir()).unwrap_or(false);
		if is_dir {
			record_one(
				tar,
				sent,
				&child_name,
				&abs,
				KindDispatch::Dir,
				true,
				cp_err,
			)?;
		} else {
			record_one(
				tar,
				sent,
				&child_name,
				&abs,
				KindDispatch::FileOrLink,
				true,
				cp_err,
			)?;
		}
	}
	Ok(())
}

/// Pack a tree through the descriptors the walk opened.
///
/// `prefix` is the archive path the tree is stored under: the descendants land
/// at `prefix/<relative path>`, and the wrapper entry for `prefix` closes the
/// tree. Without a prefix (`contents` mode) the descendants land at their
/// relative path and there is no wrapper.
///
/// The descendants first, then the wrapper: `tree_landed` walks the recorded
/// list last-to-first so a truncated upload (the tail is gone) is refused on
/// the first stat instead of the last; putting the wrapper at the end
/// preserves the ordering `append_dir_all` had.
fn pack_pinned_tree<W: Write>(
	src: &Path,
	prefix: Option<&Path>,
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
) -> Result<()> {
	// The root is the path the user named, so it is read by path. Its
	// metadata closes the tree as the wrapper entry.
	let root = std::fs::metadata(src).map_err(|e| {
		cp_err(&format!(
			"{} when getting metadata for {}",
			e,
			src.display()
		))
	})?;
	let mut failure: Option<ComposeError> = None;
	let walked = walk::visit_dir_skipping(
		src,
		|_| false,
		|_| true,
		|abs, entry| {
			match append_pinned(tar, sent, src, prefix, abs, entry) {
				Ok(()) => Ok(()),
				Err(e) => {
					// The walk stops at the first error, so the reason is kept
					// here and reported once the walk is over.
					failure = Some(e);
					Err(std::io::Error::other("cp: the entry could not be packed"))
				}
			}
		},
	);
	if let Some(e) = failure {
		return Err(e);
	}
	walked.map_err(walk_err)?;
	if let Some(name) = prefix {
		let mut header = tar::Header::new_gnu();
		header.set_metadata(&root);
		header.set_entry_type(tar::EntryType::Directory);
		header.set_size(0);
		tar.append_data(&mut header, name, std::io::empty())
			.map_err(|e| cp_err(&e.to_string()))?;
		push_entry(sent, name, SentKind::Dir)?;
	}
	Ok(())
}

/// Append one entry the descriptor walk handed over, and record it.
///
/// The bytes come from the descriptor the walk opened, never from the path
/// the entry was found at: a name that is replaced between the walk and the
/// write would otherwise pack whatever the path points at by then.
fn append_pinned<W: Write>(
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
	src: &Path,
	prefix: Option<&Path>,
	abs: &Path,
	entry: walk::Entry,
) -> Result<()> {
	let rel = abs.strip_prefix(src).map_err(|_| {
		ComposeError::Build(format!(
			"cp: walk produced path outside source: {}",
			abs.display()
		))
	})?;
	let name: PathBuf = match prefix {
		Some(p) => p.join(rel),
		None => rel.to_path_buf(),
	};
	match entry {
		walk::Entry::File(mut file) => {
			// The size comes from the descriptor that is about to be read, so
			// the recorder and the header cannot describe two different files.
			let size = file
				.metadata()
				.map_err(|e| {
					cp_err(&format!(
						"{} when getting metadata for {}",
						e,
						abs.display()
					))
				})?
				.len();
			crate::engine::context::tar_stream::append_pinned_file(tar, &name, &mut file)
				.map_err(|e| cp_err(&e.to_string()))?;
			push_entry(sent, &name, SentKind::File(size))?;
		}
		walk::Entry::Directory(metadata) => {
			let mut header = tar::Header::new_gnu();
			header.set_metadata(&metadata);
			header.set_entry_type(tar::EntryType::Directory);
			header.set_size(0);
			tar.append_data(&mut header, &name, std::io::empty())
				.map_err(|e| cp_err(&e.to_string()))?;
			push_entry(sent, &name, SentKind::Dir)?;
		}
		walk::Entry::Symlink(target) => {
			// `to_str` rejects bytes that are not valid UTF-8; mirrors the
			// `sent_entries` rule that turns a non-UTF-8 link name into an
			// error so the verification fails closed rather than confirming on
			// the lossy-rewritten name.
			let target_str = target
				.to_str()
				.ok_or_else(|| ComposeError::Build("cp: symlink link name is not UTF-8".into()))?;
			// This version of the `tar` crate's `append_link` takes a
			// pre-built Header; mirror the on-disk mode so a destination that
			// reads the entry back from the archive sees the same shape
			// `append_path_with_name` would have produced.
			let mut header = tar::Header::new_gnu();
			header.set_entry_type(tar::EntryType::Symlink);
			header.set_size(0);
			header.set_mode(0o777);
			tar.append_link(&mut header, &name, &target)
				.map_err(|e| cp_err(&e.to_string()))?;
			push_entry(sent, &name, SentKind::Link(target_str.to_string()))?;
		}
	}
	Ok(())
}

/// Map an internal error to a `cp`-category `ComposeError`. Used by the
/// shared [`super::common::record_one`] helper.
fn cp_err(msg: &str) -> ComposeError {
	ComposeError::Copy(format!("cp: {msg}"))
}

/// Map a failure of the descriptor walk to a `cp`-category error.
///
/// The walk refuses an entry whose kind it cannot confirm (a socket, a pipe,
/// a device) before the entry reaches the visitor. That keeps the wording the
/// recorder uses for the same condition; any other failure (a directory that
/// cannot be read, say) is reported as what it is.
fn walk_err(e: std::io::Error) -> ComposeError {
	if e.to_string().contains("socket, pipe or device") {
		ComposeError::Build(format!("cp: source entry has unverified kind: {e}"))
	} else {
		ComposeError::Build(format!("cp: {e}"))
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
