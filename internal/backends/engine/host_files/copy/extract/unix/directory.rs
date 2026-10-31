//! Filesystem operations relative to directory descriptors, never path prefixes.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

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

type FileIdentity = (u64, u64);

// Device and inode fields have platform-dependent integer types.
#[allow(clippy::unnecessary_cast)]
fn file_identity(stat: &fs::Stat) -> FileIdentity {
	(stat.st_dev as u64, stat.st_ino as u64)
}

/// What an extraction has created so far, kept as identities and not as open
/// descriptors: a tree of thousands of files would otherwise run out of them.
#[derive(Default)]
pub(super) struct ExtractionState {
	/// Regular files this extraction created. A hard link may only point at one.
	files: HashSet<FileIdentity>,
	/// Directories in the order they were first visited, with the mode each is
	/// to end up with once everything inside it has been extracted.
	directories: Vec<PendingDirectory>,
	directory_indices: HashMap<FileIdentity, usize>,
}

struct PendingDirectory {
	path: PathBuf,
	identity: FileIdentity,
	mode: Option<u32>,
}

impl ExtractionState {
	pub(super) fn record_file(&mut self, file: &File) -> io::Result<()> {
		self.files.insert(file_identity(&fs::fstat(file)?));
		Ok(())
	}

	/// Note a directory seen at `path` (relative to the extraction root). A
	/// repeated header only updates the mode it is to end up with.
	pub(super) fn record_directory(
		&mut self,
		dir: &Directory,
		path: &Path,
		mode: Option<u32>,
	) -> io::Result<()> {
		let identity = file_identity(&fs::fstat(&dir.0)?);
		if let Some(&index) = self.directory_indices.get(&identity) {
			if mode.is_some() {
				self.directories[index].mode = mode;
			}
		} else {
			self.directory_indices
				.insert(identity, self.directories.len());
			self.directories.push(PendingDirectory {
				path: path.to_path_buf(),
				identity,
				mode,
			});
		}
		Ok(())
	}

	/// Give every directory its final mode, the ones inside first. Each is
	/// reached again from the pinned root without following links and must be
	/// the directory that was recorded.
	pub(super) fn finish_directory_modes(
		self,
		root: &Directory,
		flattened_wrapper: Option<&OsStr>,
	) -> io::Result<()> {
		for pending in self.directories.into_iter().rev() {
			let Some(mode) = pending.mode else { continue };
			let path = match flattened_wrapper {
				Some(wrapper) => pending
					.path
					.strip_prefix(Path::new(wrapper))
					.unwrap_or(&pending.path),
				None => &pending.path,
			};
			// The removed wrapper contributes no mode to the destination root.
			if path.as_os_str().is_empty() {
				continue;
			}
			let (parent, name) = root.parent(path, false)?;
			let dir = parent.child(&name, false)?;
			if file_identity(&fs::fstat(&dir.0)?) != pending.identity {
				return Err(io::Error::other(format!(
					"cp: directory changed while it was being extracted: {}",
					pending.path.display()
				)));
			}
			dir.chmod(mode)?;
		}
		Ok(())
	}
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

	/// Link a regular file created by this extraction within the pinned root.
	///
	/// A source that existed before the extraction is refused: its inode may
	/// have other names outside the destination, and the mode in the archive
	/// is never applied to an inode the extraction did not create.
	pub(super) fn hardlink(
		&self,
		name: &OsStr,
		root: &Self,
		target: &Path,
		mode: Option<u32>,
		state: &ExtractionState,
	) -> io::Result<Option<File>> {
		let (source, leaf) = root.parent(target, false)?;
		let stat = fs::statat(&source.0, &leaf, AtFlags::SYMLINK_NOFOLLOW)?;
		if fs::FileType::from_raw_mode(stat.st_mode) != fs::FileType::RegularFile {
			return Err(io::Error::other(format!(
				"cp: hard link source is not a regular file: {}",
				target.display()
			)));
		}
		let identity = file_identity(&stat);
		if !state.files.contains(&identity) {
			return Err(io::Error::other(format!(
				"cp: hard link source was not created by this extraction: {}",
				target.display()
			)));
		}
		// No SYMLINK_FOLLOW: a leaf changed after stat can only link the
		// symlink itself, never its target.
		fs::linkat(&source.0, &leaf, &self.0, name, AtFlags::empty())?;
		let linked = fs::statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW)?;
		if fs::FileType::from_raw_mode(linked.st_mode) != fs::FileType::RegularFile
			|| file_identity(&linked) != identity
		{
			return Err(io::Error::other(format!(
				"cp: hard link source identity changed: {}",
				target.display()
			)));
		}
		if mode.is_none_or(|mode| linked.st_mode & 0o7777 == masked_mode(mode).bits()) {
			return Ok(None);
		}
		#[cfg(test)]
		before_reopen::run();
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
		// The name was opened again, so what came back is checked, not assumed:
		// a regular file with another identity is a different inode, and the mode
		// is never applied to it.
		let opened = fs::fstat(&fd)?;
		if fs::FileType::from_raw_mode(opened.st_mode) != fs::FileType::RegularFile
			|| file_identity(&opened) != identity
		{
			return Err(io::Error::other(format!(
				"cp: hard link changed while its mode was being applied: {}",
				target.display()
			)));
		}
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
	pub(crate) fn flatten(&self) -> io::Result<Option<OsString>> {
		self.flatten_after_open(|| {})
	}

	fn flatten_after_open(&self, after_open: impl FnOnce()) -> io::Result<Option<OsString>> {
		let children = self.names()?;
		if children.len() != 1 {
			return Ok(None);
		}
		let name = &children[0];
		let wrapper = match self.child(name, false) {
			Ok(dir) => dir,
			Err(e) if matches!(e.raw_os_error(), Some(libc::ENOTDIR | libc::ELOOP)) => {
				return Ok(None)
			}
			Err(e) => return Err(e),
		};
		after_open();
		for child in wrapper.names()? {
			fs::renameat(&wrapper.0, &child, &self.0, &child)?;
		}
		// unlinkat removes the directory entry itself; it never follows a link.
		fs::unlinkat(&self.0, name, AtFlags::REMOVEDIR)?;
		Ok(Some(name.clone()))
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
