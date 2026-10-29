//! Descriptor-relative traversal: no descendant path component is followed.

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use rustix::fs::{self, AtFlags, Mode, OFlags};

use super::{changed_type, io, special_file, Entry, File, Path};

const DIRECTORY: OFlags = OFlags::RDONLY
	.union(OFlags::DIRECTORY)
	.union(OFlags::CLOEXEC);

/// Pin the root and resolve each descendant relative to its open parent.
pub(super) fn walk(
	root: &Path,
	skip_dir: &impl Fn(&Path) -> bool,
	include_entry: &impl Fn(&Path) -> bool,
	visit: &mut impl FnMut(&Path, Entry) -> io::Result<()>,
) -> io::Result<()> {
	let dir: File = fs::open(root, DIRECTORY, Mode::empty())?.into();
	collect(&dir, root, skip_dir, include_entry, visit)
}

fn collect(
	dir: &File,
	path: &Path,
	skip_dir: &impl Fn(&Path) -> bool,
	include_entry: &impl Fn(&Path) -> bool,
	visit: &mut impl FnMut(&Path, Entry) -> io::Result<()>,
) -> io::Result<()> {
	let entries = fs::Dir::new(fs::openat(dir, ".", DIRECTORY, Mode::empty())?)?;
	let mut names = Vec::new();
	for entry in entries {
		let entry = entry?;
		let bytes = entry.file_name().to_bytes();
		if bytes != b"." && bytes != b".." {
			names.push(OsStr::from_bytes(bytes).to_owned());
		}
	}
	names.sort();
	for name in names {
		let child_path = path.join(&name);
		let stat = match fs::statat(dir, &name, AtFlags::SYMLINK_NOFOLLOW) {
			Ok(stat) => stat,
			// Listed a moment ago and gone now (an editor's temporary file): it is
			// not part of the context any more, which is not an error.
			Err(rustix::io::Errno::NOENT) => continue,
			Err(e) => return Err(e.into()),
		};
		let kind = fs::FileType::from_raw_mode(stat.st_mode);
		let skip = kind == fs::FileType::Directory && skip_dir(&child_path);
		let include = include_entry(&child_path);
		if !include && (skip || kind != fs::FileType::Directory) {
			continue;
		}
		#[cfg(test)]
		super::super::bump_walk_counter();
		if kind == fs::FileType::Symlink {
			let target = fs::readlinkat(dir, &name, Vec::new())?;
			// Reading a replaced link can change only link text, never read
			// target bytes. A replacement with a non-link is rejected.
			let meta = fs::statat(dir, &name, AtFlags::SYMLINK_NOFOLLOW)?;
			if fs::FileType::from_raw_mode(meta.st_mode) != fs::FileType::Symlink {
				return Err(changed_type());
			}
			visit(
				&child_path,
				Entry::Symlink(Path::new(OsStr::from_bytes(target.as_bytes())).to_path_buf()),
			)?;
			continue;
		}
		if !matches!(kind, fs::FileType::RegularFile | fs::FileType::Directory) {
			return Err(special_file(&child_path));
		}
		let flags = if kind == fs::FileType::Directory {
			DIRECTORY | OFlags::NOFOLLOW
		} else {
			OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC
		};
		let file: File = fs::openat(dir, &name, flags, Mode::empty())?.into();
		let opened = fs::fstat(&file)?;
		if fs::FileType::from_raw_mode(opened.st_mode) != kind
			|| opened.st_dev != stat.st_dev
			|| opened.st_ino != stat.st_ino
		{
			return Err(changed_type());
		}
		if kind == fs::FileType::Directory {
			if include {
				visit(&child_path, Entry::Directory(file.metadata()?))?;
			}
			if !skip {
				collect(&file, &child_path, skip_dir, include_entry, visit)?;
			}
		} else {
			visit(&child_path, Entry::File(file))?;
		}
	}
	Ok(())
}
