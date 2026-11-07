//! Which unit files in the Quadlet directory belong to a project.

use std::path::{Path, PathBuf};

use super::quadlet_dir;
use crate::ComposeError;

/// The project name recorded in a generated unit file's `# podup-owner:`
/// marker, or `None` if the file carries no such marker.
///
/// This deliberately does NOT read the `Label=podup.project=<project>` line
/// every unit builder in `crate::quadlet` also stamps (that label stays, but
/// only for its original purpose; Podman uses `podup.project` for
/// container/secret scoping at runtime). A compose service's user-supplied
/// `labels:` renders into the very same `[Section]`, in the very same
/// `Key=Value` shape, so a service declaring `labels: {podup.project:
/// other}` produces a forged `Label=podup.project=other` line that is
/// textually indistinguishable from the real one, and would be the FIRST
/// such line if it precedes the trusted stamp, defeating a scan that takes
/// the first match. The `# podup-owner: <project>` marker every unit builder
/// emits as its literal first line cannot be forged the same way: systemd
/// treats `#`-prefixed lines as comments, and compose labels only ever
/// render as `Label=key=value` entries, never as a comment line. So this is
/// the one place ownership is decided.
///
/// Only the head of the file is read, through a reader that opens without
/// following a link and without waiting for a writer, and refuses what is not a
/// regular file: the entry lives in a directory whose contents podup does not
/// control, and a FIFO there would block a plain read forever. An error when the
/// file cannot be read at all.
fn unit_owner(path: &Path) -> std::io::Result<Option<String>> {
	// The marker is the first line of the unit, so only the head is read: the
	// size of the whole unit (an escaped label can double a long value) and the
	// bytes after the header decide nothing about who owns it.
	let head = crate::filesystem::read_head_capped(path, OWNER_HEAD_BYTES)?;
	let head = String::from_utf8_lossy(&head);
	Ok(head
		.lines()
		.find_map(|line| line.strip_prefix("# podup-owner: ").map(str::to_string)))
}

/// How much of a unit is read to find its ownership marker.
const OWNER_HEAD_BYTES: usize = 4096;

/// The suffixes of the files Quadlet reads as units.
const UNIT_SUFFIXES: [&str; 8] = [
	".container",
	".build",
	".network",
	".volume",
	".pod",
	".kube",
	".image",
	".artifact",
];

fn has_unit_suffix(name: &str) -> bool {
	UNIT_SUFFIXES.iter().any(|s| name.ends_with(s))
}

/// This project's installed quadlet unit files, sorted. Drives uninstall
/// (remove) and rebuild (find `.build` units).
///
/// A file name starting with `<project>-` is only a candidate: project names
/// may themselves contain `-`, so `app-extra-web.container` also starts with
/// `app-`. Matching on that prefix alone (the old behaviour) meant
/// `uninstall -p app` matched (and `uninstall_quadlet` then stopped and
/// deleted) the sibling project `app-extra`'s units. Each candidate is
/// therefore opened and kept only when its `# podup-owner:` marker equals
/// `project` EXACTLY (see `unit_owner` above); the marker is exact by
/// construction and, unlike the `Label=podup.project=` line, cannot be
/// pre-empted by a forged line from the compose file's own `labels:`.
///
/// A candidate with no marker at all (installed before this ownership check
/// existed) cannot be proven to belong to `project`: treating "no marker" as
/// "assume it's ours" would just reopen the same hole for those legacy
/// installs. So it is left in place, not deleted, and reported via
/// `tracing::warn!` so the user can re-install (which re-marks it) or remove
/// it by hand. Quadlet-mode autostart is recent, so few if any pre-existing
/// unmarked units are expected in the field; leaving one stale file behind
/// for a user to clean up once is a far smaller cost than deleting a
/// sibling project's unit.
///
/// Only entries whose name ends in a Quadlet unit suffix are candidates, and
/// only real files: a drop-in directory (`<unit>.container.d`), a file that is
/// not a unit and a link (a dotfiles manager puts those in) are left alone,
/// because podup writes none of them. A candidate whose bytes cannot be read
/// as text is skipped with a warning too, since ownership cannot be proven
/// for it. A FIFO, a device or a socket under a unit name is an error naming
/// its path: it would hold a plain read forever, and it is never podup's.
pub(in super::super::super) fn installed_units(project: &str) -> crate::Result<Vec<PathBuf>> {
	let dir = quadlet_dir();
	let prefix = format!("{project}-");
	let mut found = Vec::new();
	if let Ok(entries) = std::fs::read_dir(&dir) {
		for entry in entries.flatten() {
			let path = entry.path();
			let name = entry.file_name().to_string_lossy().into_owned();
			if !name.starts_with(&prefix) || !has_unit_suffix(&name) {
				continue;
			}
			// `DirEntry::file_type` does not follow a link.
			match entry.file_type() {
				Ok(ty) if ty.is_file() => {}
				Ok(ty) if ty.is_dir() || ty.is_symlink() => {
					tracing::warn!(
						"{} is a {}, not a unit file podup wrote; leaving it alone",
						path.display(),
						if ty.is_dir() { "directory" } else { "link" }
					);
					continue;
				}
				_ => {
					return Err(ComposeError::Autostart(format!(
						"cannot read the quadlet unit {}: it is not a regular file (a pipe, \
						 device or socket); remove it by hand",
						path.display()
					)));
				}
			}
			match unit_owner(&path) {
				Ok(Some(owner)) if owner == project => found.push(path),
				// A sibling project's unit that happens to share this filename
				// prefix (e.g. `app-extra-web.container` when `project` is `app`);
				// the marker proves it is not ours, so it is left untouched.
				Ok(Some(_)) => {}
				Ok(None) => {
					tracing::warn!(
						"quadlet unit {} has no podup-owner ownership marker and cannot be \
						 proven to belong to '{project}'; skipping it rather than risking a \
						 sibling project's unit; re-run `podup autostart install --mode quadlet` \
						 to re-mark it, or remove it by hand if it is stale",
						path.display()
					);
				}
				Err(e) => {
					tracing::warn!(
						"cannot read the quadlet unit {}: {e}; it cannot be proven to belong to \
						 '{project}', so it is left alone",
						path.display()
					);
				}
			}
		}
	}
	found.sort();
	Ok(found)
}
