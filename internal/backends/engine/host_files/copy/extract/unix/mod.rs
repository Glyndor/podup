//! Unix extraction keeps every filesystem operation relative to open directories.

pub(super) mod directory;
mod sparse;

use std::fs::File;
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use rustix::fs::{self, Timespec, Timestamps};

use self::directory::{Directory, ExtractionState};
use super::destination::destination_refusal;
use crate::error::{ComposeError, Result};

/// Route buffered copies while retaining the selected destination descriptors.
pub(in crate::backends::engine::host_files::copy) fn extract_archive(
	bytes: &[u8],
	dst: &Path,
) -> Result<()> {
	if let Some(refusal) = destination_refusal(dst) {
		return Err(refusal);
	}
	if let Ok(root) = Directory::open_host(dst) {
		return extract(bytes, &root);
	}
	let parent_path = dst
		.parent()
		.filter(|p| !p.as_os_str().is_empty())
		.unwrap_or_else(|| Path::new("."));
	let parent = Directory::open_host(parent_path).map_err(|e| {
		if e.kind() == io::ErrorKind::NotFound {
			ComposeError::Copy(format!("cp: no such directory: {}", parent_path.display()))
		} else {
			ComposeError::Io(e)
		}
	})?;
	let name = dst
		.file_name()
		.ok_or_else(|| ComposeError::Copy("cp: destination has no filename".into()))?;
	if !super::archive::archive_contains_dir(bytes)? {
		if super::archive::has_trailing_separator(dst) {
			return Err(ComposeError::Copy(format!(
				"cp: no such directory: {}",
				dst.display()
			)));
		}
		return single_file(bytes, &parent, name, dst);
	}
	// mkdirat/openat operate on the same pinned parent. A concurrent symlink
	// replacement is rejected by child(), including after successful mkdirat.
	let root = parent.child(name, true).map_err(|e| {
		ComposeError::Copy(format!(
			"cp: cannot copy a directory to {}: {e}",
			dst.display()
		))
	})?;
	extract(bytes, &root)?;
	root.flatten().map_err(ComposeError::Io)
}

fn single_file<R: Read>(
	bytes: R,
	parent: &Directory,
	name: &std::ffi::OsStr,
	dst: &Path,
) -> Result<()> {
	let mut archive = tar::Archive::new(bytes);
	let mut written = false;
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let mut entry = entry.map_err(ComposeError::Io)?;
		if entry.header().entry_type() != tar::EntryType::Regular || written {
			return Err(ComposeError::Unsupported(format!(
				"cp: destination {} is not a directory but the source is not a single file",
				dst.display()
			)));
		}
		let mut output = parent.file(name).map_err(ComposeError::Io)?;
		copy_entry(&mut entry, &mut output)?;
		apply_mode(&output, entry.header())?;
		written = true;
	}
	if !written {
		return Err(ComposeError::Build(
			"cp: container archive was empty".into(),
		));
	}
	Ok(())
}

/// Consume a tar stream without ever reopening the extraction root by name.
pub(super) fn extract<R: Read>(src: R, root: &Directory) -> Result<()> {
	let mut archive = tar::Archive::new(src);
	let mut state = ExtractionState::default();
	for entry in archive.entries().map_err(ComposeError::Io)? {
		let mut entry = entry.map_err(ComposeError::Io)?;
		let kind = entry.header().entry_type();
		if kind.is_pax_global_extensions()
			|| kind.is_pax_local_extensions()
			|| kind.is_gnu_longname()
			|| kind.is_gnu_longlink()
		{
			continue;
		}
		let path = archive_path(&entry.path().map_err(ComposeError::Io)?)?;
		if path.as_os_str().is_empty() {
			continue;
		}
		let (parent, name) = root
			.parent_with(&path, true, |dir, walked| {
				state.record_directory(dir, walked, None)
			})
			.map_err(ComposeError::Io)?;
		let old_directory =
			entry.header().as_ustar().is_none() && entry.path_bytes().ends_with(b"/");
		if kind.is_dir() || (old_directory && !kind.is_symlink() && !kind.is_hard_link()) {
			let dir = parent.child(&name, true).map_err(ComposeError::Io)?;
			let mode = entry.header().mode().ok();
			state
				.record_directory(&dir, &path, mode)
				.map_err(ComposeError::Io)?;
			if let Some(mode) = mode {
				// Owner access is needed until all descendant entries are complete.
				dir.chmod(mode | 0o700).map_err(ComposeError::Io)?;
			}
		} else if kind.is_symlink() || kind.is_hard_link() {
			let target = entry
				.link_name()
				.map_err(ComposeError::Io)?
				.ok_or_else(|| ComposeError::Build("cp: link entry has no target".into()))?;
			if target.as_os_str().is_empty() {
				return Err(ComposeError::Build(
					"cp: link entry has an empty target".into(),
				));
			}
			if kind.is_symlink() {
				parent.symlink(&name, &target).map_err(ComposeError::Io)?;
				if let Ok(mtime) = entry.header().mtime() {
					parent
						.symlink_time(&name, mtime)
						.map_err(ComposeError::Io)?;
				}
			} else {
				if target.is_absolute() {
					return Err(escape(&target));
				}
				let linked = parent
					.hardlink(
						&name,
						root,
						&archive_path(&target)?,
						entry.header().mode().ok(),
						&state,
					)
					.map_err(ComposeError::Io)?;
				if let Some(linked) = linked {
					apply_mode(&linked, entry.header())?;
				}
			}
		} else {
			// Match tar's POSIX fallback: unrecognized entry kinds are files.
			let mut output = parent.file(&name).map_err(ComposeError::Io)?;
			if kind.is_gnu_sparse() {
				sparse::copy(&mut entry, &mut output).map_err(ComposeError::Io)?;
			} else {
				copy_entry(&mut entry, &mut output)?;
			}
			if let Ok(mtime) = entry.header().mtime() {
				let time = Timespec {
					tv_sec: mtime.max(1) as i64,
					tv_nsec: 0,
				};
				fs::futimens(
					&output,
					&Timestamps {
						last_access: time,
						last_modification: time,
					},
				)
				.map_err(|e| ComposeError::Io(e.into()))?;
			}
			apply_mode(&output, entry.header())?;
			state.record_file(&output).map_err(ComposeError::Io)?;
		}
	}
	state.finish_directory_modes(root).map_err(ComposeError::Io)
}

fn copy_entry<R: Read>(entry: &mut tar::Entry<'_, R>, output: &mut File) -> Result<()> {
	let expected = entry.size();
	let copied = io::copy(entry, output).map_err(ComposeError::Io)?;
	if copied != expected {
		return Err(ComposeError::Io(io::Error::new(
			io::ErrorKind::UnexpectedEof,
			"cp: incomplete archive entry",
		)));
	}
	Ok(())
}

fn apply_mode(output: &File, header: &tar::Header) -> Result<()> {
	if let Ok(mode) = header.mode() {
		directory::set_mode(output, mode).map_err(ComposeError::Io)?;
	}
	Ok(())
}

fn escape(path: &Path) -> ComposeError {
	ComposeError::Build(format!(
		"cp: refusing archive entry that escapes destination: {}",
		path.display()
	))
}

fn archive_path(path: &Path) -> Result<PathBuf> {
	let mut normalized = PathBuf::new();
	for component in path.components() {
		match component {
			Component::Normal(part) => normalized.push(part),
			Component::ParentDir | Component::Prefix(_) => return Err(escape(path)),
			Component::CurDir | Component::RootDir => {}
		}
	}
	Ok(normalized)
}

#[cfg(all(test, unix))]
#[path = "archive_tests.rs"]
mod tests;
