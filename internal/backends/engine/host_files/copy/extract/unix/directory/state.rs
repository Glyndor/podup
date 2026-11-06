//! The state of an extraction: what it has created so far.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use rustix::fs;

use super::{file_identity, Directory, FileIdentity};

/// What an extraction has created so far, kept as identities and not as open
/// descriptors: a tree of thousands of files would otherwise run out of them.
#[derive(Default)]
pub(crate) struct ExtractionState {
	/// Regular files this extraction created. A hard link may only point at one.
	pub(super) files: HashSet<FileIdentity>,
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
	pub(crate) fn record_file(&mut self, file: &File) -> io::Result<()> {
		self.files.insert(file_identity(&fs::fstat(file)?));
		Ok(())
	}

	/// Note a directory seen at `path` (relative to the extraction root). A
	/// repeated header only updates the mode it is to end up with.
	pub(crate) fn record_directory(
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
	pub(crate) fn finish_directory_modes(
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
