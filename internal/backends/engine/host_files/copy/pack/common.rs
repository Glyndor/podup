//! Shared walk-and-record helpers used by both `cp`'s [`super::archive::pack_path`]
//! and watch sync's [`crate::engine::watch::sync::build_sync_tar`].
//!
//! The two packers share the entry-recording shape (the `SentKind` the
//! verification compares against) and the per-entry classification rules
//! (FIFO / hard-link refusal, non-UTF-8 path refusal, link target UTF-8
//! check). They differ only in which `ComposeError` variant a failure maps to
//! — `Copy` for `cp`, `Watch` for the sync — so the helper takes an
//! error-mapping closure and the two call sites supply their own.
//!
//! Keeping the per-entry logic in one place means the recorder and the
//! archive bytes cannot drift: every entry the helper records is the same
//! entry it appends to the tar, so a sabotage on one side breaks the
//! recorded-equals-written test on both sides.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{ComposeError, Result};

use super::super::upload::verify::{SentEntry, SentKind};

/// Re-export the walkers the packers use, so the call sites read
/// `walk::walk_dir(src)` and `walk::visit_dir_skipping(src, ..)` without
/// reaching across modules.
///
/// `walk_dir` names the entries of a tree; `visit_dir_skipping` hands over
/// each entry with the descriptor it was opened with, so the caller never has
/// to reopen the path the entry was found at.
pub(in crate::backends::engine) mod walk {
	pub(in crate::backends::engine) use crate::engine::context::walk::walk_dir;
	pub(in crate::backends::engine) use crate::engine::context::walk::{visit_dir_skipping, Entry};
}

/// What kind of entry the caller wants us to append. `Dir` writes a directory
/// header; `FileOrLink` classifies the host path on disk and writes either a
/// regular-file or symlink header, recording the matching `SentKind`.
pub(in crate::backends::engine) enum KindDispatch {
	Dir,
	FileOrLink,
}

/// Append a single entry to `tar` and record its `SentEntry`.
///
/// `name` is the archive-side path; `path` is the host path. They differ only
/// for the directory wrapper and the contents branch. `map_err` translates
/// internal errors (tar IO, metadata read, link-target read) into the caller's
/// `ComposeError` variant: `cp_err` for the `cp` packer, `watch_tar` for the
/// watch-sync packer. The refusal cases (FIFO, char/block device, socket,
/// hard link; non-UTF-8 path; non-UTF-8 link target) stay as `Build` because
/// those are the same failure shape no matter which caller is asking.
///
/// `follow_link` is `-L`/`--follow-link` for `cp`. When set, a symlink is
/// classified with `metadata()` instead of `symlink_metadata()`: the link is
/// replaced by whatever it points at. A link to a regular file becomes a
/// regular file of the target's size; a link to a directory becomes a
/// directory whose contents are the target's contents. Without it, the
/// existing `symlink_metadata()` classification writes the link itself.
/// Watch sync always passes `false`: a symlink inside a watched tree would
/// otherwise copy the contents of its (possibly out-of-tree) target into the
/// container.
///
/// The classification reads the path, so it answers for the path as it is at
/// the moment of the call. That is what the root the user named asks for, and
/// what the watch sync asks for; a `cp` tree is packed through the
/// descriptors the walk opened instead (see
/// [`super::archive::pack_path`]).
///
/// Non-UTF-8 paths and link names, and unverifiable kinds (FIFO, char/block
/// device, hard link), produce errors exactly like `sent_entries` reading the
/// archive back would. The rules are the contract, not incidental.
pub(in crate::backends::engine) fn record_one<W, F>(
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
	name: &Path,
	path: &Path,
	kind: KindDispatch,
	follow_link: bool,
	map_err: F,
) -> Result<()>
where
	W: std::io::Write,
	F: Fn(&str) -> ComposeError + Copy,
{
	match kind {
		KindDispatch::Dir => {
			tar.append_dir(name, path)
				.map_err(|e| map_err(&e.to_string()))?;
			push_entry(sent, name, SentKind::Dir)?;
		}
		KindDispatch::FileOrLink => {
			if follow_link {
				// `metadata()` follows the symlink chain to its target, so
				// the classification is the target's kind, not the link's.
				let meta = std::fs::metadata(path).map_err(|e| {
					map_err(&format!(
						"{} when getting metadata for {}",
						e,
						path.display()
					))
				})?;
				let ft = meta.file_type();
				if ft.is_file() {
					// Open the target through the link. `File::open` follows
					// symlinks, so the file handle and its metadata are the
					// target's; `append_file` writes the body the handle
					// reads. `append_path_with_name` would only follow when
					// the builder's `follow_symlinks` flag is set, which is
					// not guaranteed at every call site.
					let mut file =
						std::fs::File::open(path).map_err(|e| map_err(&e.to_string()))?;
					let size = crate::engine::context::tar_stream::append_pinned_file(
						tar,
						std::path::Path::new(name),
						&mut file,
					)
					.map_err(|e| map_err(&e.to_string()))?;
					push_entry(sent, name, SentKind::File(size))?;
				} else if ft.is_dir() {
					// A symlink to a directory: the walker did not descend
					// (it does not follow `DirEntry::file_type()`), so we
					// walk the target here and append each descendant at
					// `name/<rel>`. Then the wrapper entry so the archive
					// has a directory at `name` matching what `cp -L` puts
					// in the container.
					let entries = walk::walk_dir(path).map_err(|e| map_err(&e.to_string()))?;
					for abs in entries {
						let rel = abs.strip_prefix(path).map_err(|_| {
							ComposeError::Build(format!(
								"cp: walk produced path outside target: {}",
								abs.display()
							))
						})?;
						let child_name = name.join(rel);
						let child_ft = std::fs::metadata(&abs)
							.map_err(|e| map_err(&e.to_string()))?
							.file_type();
						if child_ft.is_dir() {
							record_one(
								tar,
								sent,
								&child_name,
								&abs,
								KindDispatch::Dir,
								true,
								map_err,
							)?;
						} else {
							record_one(
								tar,
								sent,
								&child_name,
								&abs,
								KindDispatch::FileOrLink,
								true,
								map_err,
							)?;
						}
					}
					tar.append_dir(name, path)
						.map_err(|e| map_err(&e.to_string()))?;
					push_entry(sent, name, SentKind::Dir)?;
				} else {
					// FIFO, char/block device, socket, hard link, anything
					// else the destination cannot ask about: refuse up front,
					// the same way `sent_entries` would refuse on the
					// read-back side.
					return Err(ComposeError::Build(format!(
						"cp: source entry {} has unverified kind",
						path.display()
					)));
				}
			} else {
				// Classify without following symlinks: the entry is recorded
				// as a link if it is one.
				let file_type = std::fs::symlink_metadata(path)
					.map_err(|e| {
						map_err(&format!(
							"{} when getting metadata for {}",
							e,
							path.display()
						))
					})?
					.file_type();
				if file_type.is_symlink() {
					let target = std::fs::read_link(path).map_err(|e| map_err(&e.to_string()))?;
					// `to_str` rejects bytes that are not valid UTF-8; mirrors
					// the `sent_entries` rule that turns a non-UTF-8 link name
					// into an error so the verification fails closed rather than
					// confirming on the lossy-rewritten name.
					let target_str = target.to_str().ok_or_else(|| {
						ComposeError::Build("cp: symlink link name is not UTF-8".into())
					})?;
					// This version of the `tar` crate's `append_link` takes a
					// pre-built Header; mirror the on-disk mode so a destination
					// that reads the entry back from the archive sees the same
					// shape `append_path_with_name` would have produced.
					let mut header = tar::Header::new_gnu();
					header.set_entry_type(tar::EntryType::Symlink);
					header.set_size(0);
					header.set_mode(0o777);
					tar.append_link(&mut header, name, &target)
						.map_err(|e| map_err(&e.to_string()))?;
					push_entry(sent, name, SentKind::Link(target_str.to_string()))?;
				} else if file_type.is_file() {
					// One open handle for the header, the body and the size that is
					// recorded; the leaf is not followed, so a link put under the
					// name after the classification above is refused, not read.
					let mut file =
						open_without_following(path).map_err(|e| map_err(&e.to_string()))?;
					if !file
						.metadata()
						.map_err(|e| map_err(&e.to_string()))?
						.is_file()
					{
						return Err(map_err(&format!(
							"{} is no longer a regular file",
							path.display()
						)));
					}
					let size = crate::engine::context::tar_stream::append_pinned_file(
						tar,
						std::path::Path::new(name),
						&mut file,
					)
					.map_err(|e| map_err(&e.to_string()))?;
					push_entry(sent, name, SentKind::File(size))?;
				} else {
					// FIFO, char/block device, socket, hard link, anything else
					// the destination cannot ask about: refuse up front, the same
					// way `sent_entries` would refuse on the read-back side.
					return Err(ComposeError::Build(format!(
						"cp: source entry {} has unverified kind",
						path.display()
					)));
				}
			}
		}
	}
	Ok(())
}

/// Append one entry to the recorded list. The path is checked the way
/// `sent_entries` checks the archive-side path: every component must be a
/// `Component::Normal` whose `OsStr` is valid UTF-8, and an entry whose path
/// collapses to nothing (`.`) is skipped.
pub(in crate::backends::engine) fn push_entry(
	sent: &mut Vec<SentEntry>,
	name: &Path,
	kind: SentKind,
) -> Result<()> {
	let mut names = Vec::new();
	for component in name.components() {
		match component {
			std::path::Component::Normal(part) => {
				let s = part.to_str().ok_or_else(|| {
					ComposeError::Build("cp: archive entry path is not UTF-8".into())
				})?;
				names.push(s.to_string());
			}
			_ => continue,
		}
	}
	if names.is_empty() {
		return Ok(());
	}
	sent.push(SentEntry {
		path: names.join("/"),
		kind,
	});
	Ok(())
}

/// Append one entry the descriptor walk handed over, and record it. Shared by
/// `cp` and the watch sync, which differ in the error category `map_err` gives.
///
/// The bytes come from the descriptor the walk opened, never from the path
/// the entry was found at: a name that is replaced between the walk and the
/// write would otherwise pack whatever the path points at by then.
pub(in crate::backends::engine) fn append_pinned<W: Write>(
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
	src: &Path,
	prefix: Option<&Path>,
	abs: &Path,
	entry: walk::Entry,
	map_err: &dyn Fn(&str) -> ComposeError,
) -> Result<()> {
	let rel = abs.strip_prefix(src).map_err(|_| {
		map_err(&format!(
			"walk produced path outside source: {}",
			abs.display()
		))
	})?;
	let name: PathBuf = match prefix {
		Some(p) => p.join(rel),
		None => rel.to_path_buf(),
	};
	match entry {
		walk::Entry::File(mut file) => {
			// The size the header announces is the size that is recorded: the
			// helper reports it, so the two cannot come from separate answers.
			let size =
				crate::engine::context::tar_stream::append_pinned_file(tar, &name, &mut file)
					.map_err(|e| map_err(&e.to_string()))?;
			push_entry(sent, &name, SentKind::File(size))?;
		}
		walk::Entry::Directory(metadata) => {
			let mut header = tar::Header::new_gnu();
			header.set_metadata(&metadata);
			header.set_entry_type(tar::EntryType::Directory);
			header.set_size(0);
			tar.append_data(&mut header, &name, std::io::empty())
				.map_err(|e| map_err(&e.to_string()))?;
			push_entry(sent, &name, SentKind::Dir)?;
		}
		walk::Entry::Symlink(target) => {
			// `to_str` rejects bytes that are not valid UTF-8; mirrors the
			// `sent_entries` rule that turns a non-UTF-8 link name into an
			// error so the verification fails closed rather than confirming on
			// the lossy-rewritten name.
			let target_str = target
				.to_str()
				.ok_or_else(|| map_err("symlink link name is not UTF-8"))?;
			// This version of the `tar` crate's `append_link` takes a
			// pre-built Header; mirror the on-disk mode so a destination that
			// reads the entry back from the archive sees the same shape
			// `append_path_with_name` would have produced.
			let mut header = tar::Header::new_gnu();
			header.set_entry_type(tar::EntryType::Symlink);
			header.set_size(0);
			header.set_mode(0o777);
			tar.append_link(&mut header, &name, &target)
				.map_err(|e| map_err(&e.to_string()))?;
			push_entry(sent, &name, SentKind::Link(target_str.to_string()))?;
		}
	}
	Ok(())
}

/// Open `path` for reading without following a link in the last component.
fn open_without_following(path: &Path) -> std::io::Result<std::fs::File> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::OpenOptionsExt;
		std::fs::OpenOptions::new()
			.read(true)
			.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
			.open(path)
	}
	#[cfg(windows)]
	{
		use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
		// Win32 FILE_FLAG_OPEN_REPARSE_POINT: the link itself is opened, not what
		// it points at, and the attributes of the handle say whether it is one.
		const OPEN_REPARSE_POINT: u32 = 0x0020_0000;
		const REPARSE_POINT: u32 = 0x400;
		let file = std::fs::OpenOptions::new()
			.read(true)
			.custom_flags(OPEN_REPARSE_POINT)
			.open(path)?;
		if file.metadata()?.file_attributes() & REPARSE_POINT != 0 {
			return Err(std::io::Error::other(format!(
				"{} is a link and not a regular file",
				path.display()
			)));
		}
		Ok(file)
	}
	#[cfg(not(any(unix, windows)))]
	{
		std::fs::File::open(path)
	}
}

#[cfg(test)]
#[path = "common_open_tests.rs"]
mod open_tests;
