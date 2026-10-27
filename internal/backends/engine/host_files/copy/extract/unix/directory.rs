//! Filesystem operations relative to directory descriptors, never path prefixes.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path};

use rustix::fs::{self, AtFlags, Mode, OFlags};
use rustix::io::Errno;

const READ_DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
	.union(OFlags::DIRECTORY)
	.union(OFlags::NOFOLLOW)
	.union(OFlags::CLOEXEC);

// Search descriptors preserve traversal through execute-only ancestors.
#[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
const DIRECTORY_FLAGS: OFlags = READ_DIRECTORY_FLAGS.union(OFlags::PATH);
#[cfg(any(
	target_vendor = "apple",
	target_os = "netbsd",
	target_os = "solaris",
	target_os = "illumos"
))]
const DIRECTORY_FLAGS: OFlags =
	READ_DIRECTORY_FLAGS.union(OFlags::from_bits_retain(libc::O_SEARCH as _));
#[cfg(not(any(
	target_os = "linux",
	target_os = "android",
	target_os = "freebsd",
	target_vendor = "apple",
	target_os = "netbsd",
	target_os = "solaris",
	target_os = "illumos"
)))]
const DIRECTORY_FLAGS: OFlags = READ_DIRECTORY_FLAGS;

/// A directory whose identity survives replacement of its path components.
pub(crate) struct Directory(File);

impl Directory {
	/// Open each host component separately, preserving physical `..` semantics.
	pub(crate) fn open_host(path: &Path) -> io::Result<Self> {
		let root = Self(fs::open("/", DIRECTORY_FLAGS, Mode::empty())?.into());
		let mut dir = Self(
			fs::open(
				if path.is_absolute() { "/" } else { "." },
				DIRECTORY_FLAGS,
				Mode::empty(),
			)?
			.into(),
		);
		for component in path.components() {
			match component {
				Component::Normal(name) => dir = dir.host_child(name, &root)?,
				Component::ParentDir => dir = dir.child(OsStr::new(".."), false)?,
				Component::CurDir | Component::RootDir => {}
				Component::Prefix(_) => return Err(io::Error::other("cp: invalid Unix prefix")),
			}
		}
		Ok(dir)
	}

	fn host_child(&self, name: &OsStr, root: &Self) -> io::Result<Self> {
		match self.child(name, false) {
			Ok(dir) => Ok(dir),
			Err(error) => {
				// Only a root-owned link directly in a non-writable, root-owned
				// system root may be followed (/tmp on macOS, /home on Atomic).
				// Compare directory identities, including for relative `..` paths.
				let parent = fs::fstat(&self.0)?;
				let system = fs::fstat(&root.0)?;
				let link = fs::statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW)?;
				if parent.st_dev != system.st_dev
					|| parent.st_ino != system.st_ino
					|| parent.st_uid != 0
					|| parent.st_mode & 0o022 != 0
					|| link.st_uid != 0
					|| fs::FileType::from_raw_mode(link.st_mode) != fs::FileType::Symlink
				{
					return Err(error);
				}
				Ok(Self(
					fs::openat(
						&self.0,
						name,
						DIRECTORY_FLAGS & !OFlags::NOFOLLOW,
						Mode::empty(),
					)?
					.into(),
				))
			}
		}
	}

	/// Open a single child without following links, optionally creating it.
	pub(crate) fn child(&self, name: &OsStr, create: bool) -> io::Result<Self> {
		if create {
			match fs::mkdirat(&self.0, name, Mode::from_raw_mode(0o755)) {
				Ok(()) | Err(Errno::EXIST) => {}
				Err(e) => return Err(e.into()),
			}
		}
		Ok(Self(
			fs::openat(&self.0, name, DIRECTORY_FLAGS, Mode::empty())?.into(),
		))
	}

	/// Resolve only archive-normalized names, rooted at this descriptor.
	pub(crate) fn parent(&self, path: &Path, create: bool) -> io::Result<(Self, OsString)> {
		let mut parts = path.iter().collect::<Vec<_>>();
		let name = parts
			.pop()
			.ok_or_else(|| io::Error::other("cp: archive entry has no name"))?
			.to_owned();
		let mut dir = self.child(OsStr::new("."), false)?;
		for part in parts {
			dir = dir.child(part, create)?;
		}
		Ok((dir, name))
	}

	/// Create a fresh regular file; never truncate a symlink or shared inode.
	pub(crate) fn file(&self, name: &OsStr) -> io::Result<File> {
		let open = || {
			fs::openat(
				&self.0,
				name,
				OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
				Mode::from_raw_mode(0o666),
			)
		};
		match open() {
			Ok(fd) => Ok(fd.into()),
			Err(Errno::EXIST) => {
				self.unlink(name)?;
				Ok(open()?.into())
			}
			Err(e) => Err(e.into()),
		}
	}

	fn unlink(&self, name: &OsStr) -> io::Result<()> {
		match fs::unlinkat(&self.0, name, AtFlags::empty()) {
			Ok(()) | Err(Errno::NOENT) => Ok(()),
			Err(e) => Err(e.into()),
		}
	}

	/// Store a symlink as data; it will never be traversed by later operations.
	pub(crate) fn symlink(&self, name: &OsStr, target: &Path) -> io::Result<()> {
		match fs::symlinkat(target, &self.0, name) {
			Ok(()) => Ok(()),
			Err(Errno::EXIST) => {
				self.unlink(name)?;
				Ok(fs::symlinkat(target, &self.0, name)?)
			}
			Err(e) => Err(e.into()),
		}
	}

	/// Preserve a link's timestamp without following a replacement symlink.
	pub(crate) fn symlink_time(&self, name: &OsStr, mtime: u64) -> io::Result<()> {
		let time = fs::Timespec {
			tv_sec: mtime.max(1) as i64,
			tv_nsec: 0,
		};
		Ok(fs::utimensat(
			&self.0,
			name,
			&fs::Timestamps {
				last_access: time,
				last_modification: time,
			},
			AtFlags::SYMLINK_NOFOLLOW,
		)?)
	}

	/// Link a regular source within the pinned root, without following its leaf.
	pub(crate) fn hardlink(
		&self,
		name: &OsStr,
		root: &Self,
		target: &Path,
		mode: Option<u32>,
	) -> io::Result<Option<File>> {
		let (source, leaf) = root.parent(target, false)?;
		let stat = fs::statat(&source.0, &leaf, AtFlags::SYMLINK_NOFOLLOW)?;
		if fs::FileType::from_raw_mode(stat.st_mode) != fs::FileType::RegularFile {
			return Err(io::Error::other(
				"cp: hard link source is not a regular file",
			));
		}
		// No SYMLINK_FOLLOW: a leaf changed after stat can only link the
		// symlink itself, never its target. Later writes replace the inode.
		fs::linkat(&source.0, &leaf, &self.0, name, AtFlags::empty())?;
		let linked = fs::statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW)?;
		if fs::FileType::from_raw_mode(linked.st_mode) != fs::FileType::RegularFile {
			return Err(io::Error::other("cp: hard link source changed type"));
		}
		if mode.is_none_or(|mode| linked.st_mode & 0o7777 == masked_mode(mode).bits()) {
			return Ok(None);
		}
		let open = |access| {
			fs::openat(
				&self.0,
				name,
				access | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
				Mode::empty(),
			)
		};
		let fd = match open(OFlags::RDONLY) {
			Err(Errno::ACCESS) => open(OFlags::WRONLY)?,
			other => other?,
		};
		Ok(Some(fd.into()))
	}

	/// Apply sanitized permissions to the opened directory itself.
	pub(crate) fn chmod(&self, mode: u32) -> io::Result<()> {
		if fs::fstat(&self.0)?.st_mode & 0o7777 == masked_mode(mode).bits() {
			return Ok(());
		}
		// `.` names the pinned inode itself and cannot be replaced by a
		// symlink. chmodat avoids reopening an execute-only directory for read.
		Ok(fs::chmodat(
			&self.0,
			".",
			masked_mode(mode),
			AtFlags::empty(),
		)?)
	}

	fn names(&self) -> io::Result<Vec<OsString>> {
		let fd = fs::openat(&self.0, ".", READ_DIRECTORY_FLAGS, Mode::empty())?;
		let entries = fs::Dir::new(fd)?;
		let mut names = Vec::new();
		for entry in entries {
			let entry = entry?;
			let bytes = entry.file_name().to_bytes();
			if bytes != b"." && bytes != b".." {
				names.push(OsStr::from_bytes(bytes).to_owned());
			}
		}
		Ok(names)
	}

	/// Collapse the single wrapper through pinned source and target directories.
	pub(crate) fn flatten(&self) -> io::Result<()> {
		self.flatten_after_open(|| {})
	}

	fn flatten_after_open(&self, after_open: impl FnOnce()) -> io::Result<()> {
		let children = self.names()?;
		if children.len() != 1 {
			return Ok(());
		}
		let name = &children[0];
		let wrapper = match self.child(name, false) {
			Ok(dir) => dir,
			Err(e) if matches!(e.raw_os_error(), Some(libc::ENOTDIR | libc::ELOOP)) => {
				return Ok(())
			}
			Err(e) => return Err(e),
		};
		after_open();
		for child in wrapper.names()? {
			fs::renameat(&wrapper.0, &child, &self.0, &child)?;
		}
		// unlinkat removes the directory entry itself; it never follows a link.
		Ok(fs::unlinkat(&self.0, name, AtFlags::REMOVEDIR)?)
	}
}

// mode_t is narrower than the tar header's u32 on some BSD targets.
#[allow(clippy::unnecessary_cast)]
fn masked_mode(mode: u32) -> Mode {
	Mode::from_raw_mode((mode & 0o755) as fs::RawMode)
}

/// Sanitize modes on an open inode; no path lookup occurs during chmod.
pub(super) fn set_mode(file: &File, mode: u32) -> io::Result<()> {
	Ok(fs::fchmod(file, masked_mode(mode))?)
}

#[cfg(all(test, unix))]
#[path = "directory_tests.rs"]
mod tests;
