//! Shared walk-and-record helpers used by both `cp`'s [`super::archive_pack::pack_path`]
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

use std::path::Path;

use crate::error::{ComposeError, Result};

use super::verify::{SentEntry, SentKind};

/// Re-export the directory walker the two walks use, so the call sites read
/// `walk::walk_dir(src)` without reaching across modules.
pub(in crate::engine) mod walk {
	pub(in crate::engine) use crate::engine::walk::walk_dir;
}

/// What kind of entry the caller wants us to append. `Dir` writes a directory
/// header; `FileOrLink` classifies the host path on disk and writes either a
/// regular-file or symlink header, recording the matching `SentKind`.
pub(in crate::engine) enum KindDispatch {
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
/// Non-UTF-8 paths and link names, and unverifiable kinds (FIFO, char/block
/// device, hard link), produce errors exactly like `sent_entries` reading the
/// archive back would. The rules are the contract, not incidental.
pub(in crate::engine) fn record_one<W, F>(
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
					let size = meta.len();
					// Open the target through the link. `File::open` follows
					// symlinks, so the file handle and its metadata are the
					// target's; `append_file` writes the body the handle
					// reads. `append_path_with_name` would only follow when
					// the builder's `follow_symlinks` flag is set, which is
					// not guaranteed at every call site.
					let mut file =
						std::fs::File::open(path).map_err(|e| map_err(&e.to_string()))?;
					tar.append_file(name, &mut file)
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
					let size = std::fs::metadata(path)
						.map_err(|e| map_err(&e.to_string()))?
						.len();
					tar.append_path_with_name(path, name)
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
pub(in crate::engine) fn push_entry(
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
