//! The drop-in each installed container and pod service carries, in
//! `~/.config/systemd/user/<service>.d/`. It replaces the cleanup by name that Quadlet writes with a
//! cleanup of what the unit started; see [`crate::quadlet::drop_in`] and
//! [`crate::quadlet::pod_drop_in`] for what they hold.
//!
//! A drop-in is podup's only when its first line is the ownership marker of the project, the same
//! mark the units carry. One that is someone else's, carries no marker, or is not a regular file, is
//! refused before anything is written, and left alone, with a warning, when units are removed.

use std::path::{Path, PathBuf};

use super::super::install::files::{check_destination, private_dir, publish_unit};
use super::super::paths::unit_dir;
use crate::quadlet::{self, QuadletUnit};
use crate::ComposeError;

/// One drop-in to publish.
pub(super) struct DropIn {
	/// The service it belongs to, such as `proj-web.service`.
	service: String,
	/// What the file holds.
	pub(super) contents: String,
}

impl DropIn {
	/// `<service>.d/<name>`, relative to the user's systemd unit directory.
	pub(super) fn relative(&self) -> String {
		format!("{}.d/{}", self.service, quadlet::DROP_IN_NAME)
	}

	fn path(&self) -> PathBuf {
		path_for(&self.service)
	}
}

fn path_for(service: &str) -> PathBuf {
	unit_dir()
		.join(format!("{service}.d"))
		.join(quadlet::DROP_IN_NAME)
}

/// The service of a unit file that carries a drop-in: `<stem>.service` for a container,
/// `<stem>-pod.service` for a pod; `None` for the others.
pub(super) fn service_of(filename: &str) -> Option<String> {
	if let Some(stem) = filename.strip_suffix(".container") {
		return Some(format!("{stem}.service"));
	}
	filename
		.strip_suffix(".pod")
		.map(|stem| format!("{stem}-pod.service"))
}

/// The drop-ins of the container and pod units among `units`, one per service.
pub(super) fn plan(project: &str, units: &[QuadletUnit]) -> Vec<DropIn> {
	units
		.iter()
		.filter_map(|u| Some((service_of(&u.filename)?, u)))
		.map(|(service, u)| DropIn {
			service,
			contents: if u.filename.ends_with(".pod") {
				quadlet::pod_drop_in(project, &u.contents)
			} else {
				quadlet::drop_in(project, &u.contents)
			},
		})
		.collect()
}

/// The project in the marker of the drop-in at `path`, when there is a file there.
fn owner(path: &Path) -> std::io::Result<Option<Option<String>>> {
	match std::fs::symlink_metadata(path) {
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
		Err(e) => return Err(e),
		Ok(_) => check_destination(path)?,
	}
	let head = crate::filesystem::read_head_capped(path, 4096)?;
	let head = String::from_utf8_lossy(&head);
	Ok(Some(head.lines().next().and_then(|line| {
		line.strip_prefix("# podup-owner: ").map(str::to_string)
	})))
}

fn cannot(what: &str, path: &Path, e: std::io::Error) -> ComposeError {
	ComposeError::Autostart(format!("cannot {what} {}: {e}", path.display()))
}

/// Refuse, before anything is written, a drop-in directory that is not safe to write into, and a
/// destination that holds a drop-in that is not this project's or is not a regular file.
pub(super) fn check(project: &str, drop_ins: &[DropIn]) -> crate::Result<()> {
	if drop_ins.is_empty() {
		return Ok(());
	}
	let safe = |dir: &Path| {
		crate::backends::unit_directory::check_private(dir).map_err(|e| cannot("use", dir, e))
	};
	safe(&unit_dir())?;
	for drop_in in drop_ins {
		let path = drop_in.path();
		if let Some(dir) = path.parent().filter(|d| d.symlink_metadata().is_ok()) {
			safe(dir)?;
		}
		let refuse = |why: String| {
			Err(ComposeError::Autostart(format!(
				"refusing to overwrite {}: {why}",
				path.display()
			)))
		};
		match owner(&path).map_err(|e| cannot("read", &path, e))? {
			Some(Some(other)) if other != project => {
				return refuse(format!("it belongs to project '{other}', not '{project}'"));
			}
			Some(None) => {
				return refuse(
					"it carries no podup ownership marker; move it out of the way".to_string(),
				);
			}
			_ => {}
		}
	}
	Ok(())
}

/// Publish the drop-ins, each private to its owner, and return the paths written.
pub(super) fn write(drop_ins: &[DropIn]) -> crate::Result<Vec<PathBuf>> {
	let mut written = Vec::with_capacity(drop_ins.len());
	for drop_in in drop_ins {
		let path = drop_in.path();
		let dir = path.parent().unwrap_or_else(|| Path::new("."));
		private_dir(dir).map_err(|e| cannot("create", dir, e))?;
		publish_unit(&path, drop_in.contents.as_bytes()).map_err(|e| cannot("write", &path, e))?;
		written.push(path);
	}
	Ok(written)
}

/// Remove the drop-in of `service` when it is this project's, and its directory when nothing else is
/// in it. A drop-in that is not there is nothing to remove. One that cannot be proven to be this
/// project's (a link, a file that cannot be read, one without a marker) is left where it is with a
/// warning: stopping there would leave the rest of the units behind.
pub(super) fn remove(project: &str, service: &str) -> crate::Result<()> {
	let path = path_for(service);
	match owner(&path) {
		Ok(Some(Some(found))) if found == project => {}
		Ok(None) | Ok(Some(Some(_))) => return Ok(()),
		Ok(Some(None)) => {
			tracing::warn!(
				"leaving {} where it is: it carries no podup ownership marker",
				path.display()
			);
			return Ok(());
		}
		Err(e) => {
			tracing::warn!("leaving {} where it is: {e}", path.display());
			return Ok(());
		}
	}
	std::fs::remove_file(&path).map_err(|e| cannot("remove", &path, e))?;
	crate::ui::errln!("podup: removed {}", path.display())?;
	if let Some(dir) = path.parent() {
		// Another drop-in of the user's keeps the directory; that is not a failure.
		let _ = std::fs::remove_dir(dir);
	}
	Ok(())
}
