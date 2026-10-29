//! Hold directories without write/delete sharing while resolving child paths.

use std::fs::OpenOptions;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

use super::{changed_type, io, Entry, File, Path};

// Win32 FILE_FLAG_BACKUP_SEMANTICS and FILE_FLAG_OPEN_REPARSE_POINT.
const DIRECTORY: u32 = 0x0200_0000;
const NOFOLLOW: u32 = 0x0020_0000;
const SHARE_READ: u32 = 1;
const SHARE_WRITE: u32 = 2;
const REPARSE_POINT: u32 = 0x400;

/// `share` is what other handles may do while this one is open. A directory is
/// pinned against writers and deleters so a path lookup cannot be redirected; a
/// file is read while others may still hold it open for writing (a log in
/// append mode), which is how the build context always read it.
fn open(path: &Path, flags: u32, share: u32) -> io::Result<File> {
	OpenOptions::new()
		.read(true)
		.share_mode(share)
		.custom_flags(flags)
		.open(path)
}

/// Pin the canonical root's ancestors before resolving descendant paths.
pub(super) fn walk(
	root: &Path,
	skip_dir: &impl Fn(&Path) -> bool,
	include_entry: &impl Fn(&Path) -> bool,
	visit: &mut impl FnMut(&Path, Entry) -> io::Result<()>,
) -> io::Result<()> {
	// Resolve the explicitly selected root once, then pin all its ancestors
	// too: otherwise replacing an ancestor could redirect a child path lookup.
	let resolved = std::fs::canonicalize(root)?;
	let mut ancestors = Vec::new();
	for path in resolved
		.ancestors()
		.filter(|p| p.has_root())
		.collect::<Vec<_>>()
		.into_iter()
		.rev()
	{
		let file = open(path, DIRECTORY | NOFOLLOW, SHARE_READ)?;
		let metadata = file.metadata()?;
		if !metadata.is_dir() || metadata.file_attributes() & REPARSE_POINT != 0 {
			return Err(changed_type());
		}
		ancestors.push(file);
	}
	let dir = ancestors.last().ok_or_else(changed_type)?;
	// Keep visitor paths relative to the caller's root while lookups use the
	// pinned canonical spelling.
	let caller_path = |path: &Path| -> io::Result<std::path::PathBuf> {
		let rel = path.strip_prefix(&resolved).map_err(|_| changed_type())?;
		Ok(root.join(rel))
	};
	let skip = |path: &Path| caller_path(path).is_ok_and(|path| skip_dir(&path));
	let include = |path: &Path| caller_path(path).is_ok_and(|path| include_entry(&path));
	collect(dir, &resolved, &skip, &include, &mut |path, entry| {
		let rel = path.strip_prefix(&resolved).map_err(|_| changed_type())?;
		visit(&root.join(rel), entry)
	})
}

fn collect(
	_dir: &File,
	path: &Path,
	skip_dir: &impl Fn(&Path) -> bool,
	include_entry: &impl Fn(&Path) -> bool,
	visit: &mut impl FnMut(&Path, Entry) -> io::Result<()>,
) -> io::Result<()> {
	let mut entries = std::fs::read_dir(path)?.collect::<io::Result<Vec<_>>>()?;
	entries.sort_by_key(|entry| entry.file_name());
	for entry in entries {
		let path = entry.path();
		let kind = entry.file_type()?;
		let skip = kind.is_dir() && skip_dir(&path);
		let include = include_entry(&path);
		if !include && (skip || !kind.is_dir()) {
			continue;
		}
		#[cfg(test)]
		super::super::bump_walk_counter();
		if kind.is_symlink() {
			let target = std::fs::read_link(&path)?;
			let meta = path.symlink_metadata()?;
			if !meta.file_type().is_symlink() {
				return Err(changed_type());
			}
			visit(&path, Entry::Symlink(target))?;
			continue;
		}
		let share = if kind.is_dir() {
			SHARE_READ
		} else {
			SHARE_READ | SHARE_WRITE
		};
		let file = open(&path, DIRECTORY | NOFOLLOW, share)?;
		let meta = file.metadata()?;
		if meta.file_type() != kind
			|| meta.file_attributes() & REPARSE_POINT != 0
			|| (!meta.is_dir() && !meta.is_file())
		{
			return Err(changed_type());
		}
		if meta.is_dir() {
			if include {
				visit(&path, Entry::Directory(meta))?;
			}
			if !skip {
				collect(&file, &path, skip_dir, include_entry, visit)?;
			}
		} else {
			visit(&path, Entry::File(file))?;
		}
	}
	Ok(())
}
