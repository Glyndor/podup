//! Filesystem operations relative to directory descriptors, never path prefixes.
//!
//! The descriptor is the only thing an operation names: a path is walked
//! component by component, each one opened and kept, so a component replaced
//! by a symlink halfway through cannot redirect the rest of the walk.

mod entries;
mod flatten;
mod links;
mod modes;
mod state;

pub(super) use modes::set_mode;
pub(super) use state::ExtractionState;

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use rustix::fs::{self, AtFlags, Mode, OFlags};
use rustix::io::Errno;

use self::modes::masked_mode;

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

/// Test seam: a closure run between linking a hard link and opening it again,
/// to put another file under its name at exactly that point.
#[cfg(test)]
pub(super) mod before_reopen {
	use std::cell::RefCell;

	thread_local! {
		static HOOK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
	}

	pub(in crate::backends::engine::host_files::copy::extract) fn set(hook: impl Fn() + 'static) {
		HOOK.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
	}

	pub(super) fn run() {
		HOOK.with(|h| {
			if let Some(hook) = h.borrow_mut().take() {
				hook();
			}
		});
	}
}

/// A directory whose identity survives replacement of its path components.
pub(crate) struct Directory(File);

pub(super) type FileIdentity = (u64, u64);

// Device and inode fields have platform-dependent integer types.
#[allow(clippy::unnecessary_cast)]
pub(super) fn file_identity(stat: &fs::Stat) -> FileIdentity {
	(stat.st_dev as u64, stat.st_ino as u64)
}

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
		self.parent_with(path, create, |_, _| Ok(()))
	}

	/// Visit each parent in traversal order while its descriptor is pinned.
	pub(super) fn parent_with(
		&self,
		path: &Path,
		create: bool,
		mut visit: impl FnMut(&Self, &Path) -> io::Result<()>,
	) -> io::Result<(Self, OsString)> {
		let mut parts = path.iter().collect::<Vec<_>>();
		let name = parts
			.pop()
			.ok_or_else(|| io::Error::other("cp: archive entry has no name"))?
			.to_owned();
		let mut dir = self.child(OsStr::new("."), false)?;
		let mut walked = PathBuf::new();
		for part in parts {
			dir = dir.child(part, create)?;
			walked.push(part);
			visit(&dir, &walked)?;
		}
		Ok((dir, name))
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
}

#[cfg(all(test, unix))]
#[path = "../directory_tests.rs"]
mod tests;
