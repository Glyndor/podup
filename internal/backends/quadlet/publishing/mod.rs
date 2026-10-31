//! Validate the complete batch before publishing individual private units.

use super::{unit_file, QuadletUnit};

/// The `# podup-owner: <project>` marker a unit carries as its literal first
/// line, if present.
fn marker_owner(contents: &str) -> Option<&str> {
	contents
		.lines()
		.find_map(|line| line.strip_prefix("# podup-owner: "))
}

/// Refuse to overwrite a unit that belongs to another project.
///
/// Unit stems are `{project}-{service}`, so project `app` with service
/// `extra-web` and project `app-extra` with service `web` both produce
/// `app-extra-web.container`. Uninstall already resolves that ambiguity by
/// reading the ownership marker, but writing had no counterpart: installing one
/// project would overwrite the other's unit *and re-stamp the marker*, so the
/// next `uninstall` of the wrong project would delete it.
///
/// An existing file with no marker is left to be overwritten with a warning
/// rather than refused: it is either a unit from a podup old enough to predate
/// the marker, or a foreign file, and hard-failing would strand upgrades.
fn guard_existing_owner(
	path: &std::path::Path,
	filename: &str,
	contents: &str,
) -> std::io::Result<()> {
	let existing = match unit_file::inspect_existing(path)? {
		unit_file::Existing::Absent => return Ok(()),
		unit_file::Existing::Link => {
			tracing::warn!("replacing {filename}, which is a symbolic link");
			return Ok(());
		}
		unit_file::Existing::File(text) => text.unwrap_or_default(),
	};
	match (marker_owner(&existing), marker_owner(contents)) {
		(Some(existing_owner), Some(new_owner)) if existing_owner != new_owner => {
			Err(std::io::Error::new(
				std::io::ErrorKind::AlreadyExists,
				format!(
					"refusing to overwrite {filename}: it belongs to project '{existing_owner}', not '{new_owner}'"
				),
			))
		}
		(None, _) => {
			tracing::warn!("overwriting {filename}, which carries no podup ownership marker");
			Ok(())
		}
		_ => Ok(()),
	}
}

/// Write a unit private to its owner.
///
/// Units render each `environment:` entry as an `Environment=KEY=VALUE` line,
/// so a compose file's database password ends up in this file verbatim. The
/// default umask would leave it world-readable; systemd reads user units as the
/// owning user, so 0600 costs nothing. The file is written to a new private file
/// and renamed into place, so a link or a file with looser permissions at the
/// name is replaced rather than written through or kept.
fn write_unit_file(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
	unit_file::write_private(path, contents)
}

/// Write `units` into `dir`, creating it if needed, and return the paths written
/// in order. Defense in depth: refuse any unit whose file name is not a plain path
/// component. The library already sanitizes stems, but a write target must never
/// contain a separator, `.` or `..` that could escape `dir`. Shared by
/// `generate quadlet -o` and `autostart --mode quadlet`, so both place units
/// through the identical safety check.
pub fn write_units(
	dir: &std::path::Path,
	units: &[QuadletUnit],
) -> std::io::Result<Vec<std::path::PathBuf>> {
	crate::backends::unit_directory::create(dir)?;
	crate::backends::unit_directory::warn_if_shared(dir)?;
	let mut written = Vec::with_capacity(units.len());
	for unit in units {
		if std::path::Path::new(&unit.filename).file_name()
			!= Some(std::ffi::OsStr::new(&unit.filename))
		{
			return Err(std::io::Error::new(
				std::io::ErrorKind::InvalidInput,
				format!("refusing unsafe quadlet unit file name: {}", unit.filename),
			));
		}
		let path = dir.join(&unit.filename);
		guard_existing_owner(&path, &unit.filename, &unit.contents)?;
		written.push(path);
	}
	// Predictable type/owner failures must not publish even the first unit.
	for (unit, path) in units.iter().zip(&written) {
		write_unit_file(path, &unit.contents)?;
	}
	Ok(written)
}

/// Write to the implicit autostart directory only when it is safe to trust.
pub(crate) fn write_autostart_units(
	dir: &std::path::Path,
	units: &[QuadletUnit],
) -> std::io::Result<Vec<std::path::PathBuf>> {
	crate::backends::unit_directory::require_private(dir)?;
	write_units(dir, units)
}
