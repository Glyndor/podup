//! The `generate quadlet` command: turn the compose file into Quadlet units and
//! either write them to a directory or print them to stdout.

use std::path::Path;

/// Quadlet units are systemd unit files; they only run on Linux hosts (where
/// systemd consumes them from `~/.config/containers/systemd/`). Generating them
/// on macOS/Windows is legitimate (e.g. to deploy to a remote Linux host), so
/// this returns an advisory string rather than blocking. `os` is
/// [`std::env::consts::OS`]; the function is pure so every platform's branch is
/// testable in a single run.
fn quadlet_platform_advisory(os: &str) -> Option<String> {
	(os != "linux").then(|| {
		"quadlet units require systemd (Linux); generated files will not run on this host"
			.to_string()
	})
}

/// Generate Quadlet units from the compose file and either write them to a
/// directory or print them to stdout. Warnings about unmapped fields go to
/// stderr so stdout stays clean for piping.
pub(crate) fn write_quadlet(
	file: &podup::compose::types::ComposeFile,
	project: &str,
	output: Option<&Path>,
) -> podup::Result<()> {
	// Validate the dependency graph before emitting any unit. A cyclic `depends_on`
	// would otherwise produce units with mutual `After=`/`Requires=`, which systemd
	// cannot order; a dangling required dependency would target a non-existent
	// `.service`. `resolve_order` rejects both, matching the mutating commands.
	podup::resolve_order(file)?;
	let result = podup::quadlet::generate(file, project);
	if let Some(dup) = result.duplicate_filename() {
		return Err(std::io::Error::new(
			std::io::ErrorKind::InvalidInput,
			format!(
				"quadlet: two resources map to the same unit file {dup:?}; \
				 rename one so their names do not collide after sanitization"
			),
		)
		.into());
	}
	if let Some(advisory) = quadlet_platform_advisory(std::env::consts::OS) {
		eprintln!("podup: warning: {advisory}");
	}
	for warning in &result.warnings {
		eprintln!("podup: warning: {warning}");
	}
	match output {
		Some(dir) => {
			std::fs::create_dir_all(dir)?;
			for unit in &result.units {
				// Defense in depth: the unit stem is already sanitized in the
				// library, but never write a unit whose name is anything but a
				// plain file inside `dir` (rejects separators, `.` and `..`).
				if Path::new(&unit.filename).file_name()
					!= Some(std::ffi::OsStr::new(&unit.filename))
				{
					return Err(std::io::Error::new(
						std::io::ErrorKind::InvalidInput,
						format!("refusing unsafe quadlet unit file name: {}", unit.filename),
					)
					.into());
				}
				let path = dir.join(&unit.filename);
				std::fs::write(&path, &unit.contents)?;
				println!("wrote {}", path.display());
			}
		}
		None => {
			for unit in &result.units {
				println!("# {}", unit.filename);
				print!("{}", unit.contents);
				println!();
			}
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::{quadlet_platform_advisory, write_quadlet};

	#[test]
	fn quadlet_advisory_only_on_non_linux() {
		assert_eq!(quadlet_platform_advisory("linux"), None);
		for os in ["macos", "windows", "freebsd"] {
			let msg = quadlet_platform_advisory(os).expect("non-linux host warns");
			assert!(msg.contains("systemd"), "advisory names the requirement");
		}
	}

	#[test]
	fn cyclic_depends_on_is_rejected_before_emitting_units() {
		// A `depends_on` cycle must error rather than emit units with mutual
		// `After=`/`Requires=`; the check runs before any file I/O so `output: None`
		// is safe here.
		let file = podup::parse_str(
			"services:\n  a:\n    image: x\n    depends_on: [b]\n  b:\n    image: y\n    depends_on: [a]\n",
		)
		.unwrap();
		let err = write_quadlet(&file, "proj", None).unwrap_err();
		assert!(matches!(err, podup::ComposeError::CircularDependency(_)));
	}
}
