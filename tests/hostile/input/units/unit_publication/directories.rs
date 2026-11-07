use std::fs;
use std::os::unix::fs::{symlink, MetadataExt};

use super::support::{chmod, mode, stderr, Workspace};

#[test]
fn autostart_creates_private_directories_under_umask_zero() {
	for (kind, relative, unit) in [
		("service", "systemd/user", "podup-f6.service"),
		("quadlet", "containers/systemd", "f6-web.container"),
	] {
		let w = Workspace::new();
		let out = w.install(kind);
		let dir = w.config.join(relative);
		assert!(dir.join(unit).is_file(), "{kind}: {}", stderr(&out));
		for path in [&w.config, dir.parent().unwrap(), &dir] {
			assert_eq!(mode(path), 0o700, "{kind}: {}", path.display());
		}
	}
}

#[test]
fn explicit_export_creates_private_directories_under_umask_zero() {
	let w = Workspace::new();
	let dir = w.root.path().join("new/units");
	let out = w.export(&dir);
	assert!(out.status.success(), "{}", stderr(&out));
	assert!(dir.join("f6-web.container").is_file());
	assert_eq!(mode(&dir), 0o700);
	assert_eq!(mode(dir.parent().unwrap()), 0o700);
}

#[test]
fn autostart_rejects_other_writable_existing_directories_without_chmod() {
	for kind in ["service", "quadlet"] {
		for bits in [0o777, 0o707, 0o702] {
			let w = Workspace::new();
			let relative = if kind == "service" {
				"systemd/user"
			} else {
				"containers/systemd"
			};
			let dir = w.config.join(relative);
			fs::create_dir_all(&dir).unwrap();
			chmod(&dir, bits);
			fs::write(dir.join("keep"), b"previous bytes").unwrap();
			let out = w.install(kind);
			assert!(!out.status.success());
			assert_eq!(
				fs::read_dir(&dir).unwrap().count(),
				1,
				"{kind}: {}",
				stderr(&out)
			);
			assert_eq!(fs::read(dir.join("keep")).unwrap(), b"previous bytes");
			assert_eq!(mode(&dir), bits);
			assert!(
				stderr(&out).contains("writable by others"),
				"{}",
				stderr(&out)
			);
			chmod(&dir, 0o750);
			let out = w.install(kind);
			assert!(
				stderr(&out).contains("podup: wrote"),
				"positive control: {}",
				stderr(&out)
			);
			assert_eq!(mode(&dir), 0o750);
		}
	}
}

#[test]
fn explicit_export_warns_for_shared_directory_and_keeps_its_mode() {
	for bits in [0o777, 0o770, 0o702, 0o750] {
		let w = Workspace::new();
		let dir = w.root.path().join("out");
		fs::create_dir(&dir).unwrap();
		chmod(&dir, bits);
		let out = w.export(&dir);
		assert!(out.status.success(), "{}", stderr(&out));
		assert!(dir.join("f6-web.container").is_file());
		assert_eq!(mode(&dir), bits);
		let warning = stderr(&out);
		assert_eq!(
			warning.contains("podup: warning:"),
			bits != 0o750,
			"{warning}"
		);
		if bits != 0o750 {
			assert!(warning.contains("writable by group or others"), "{warning}");
		}
	}
}

#[test]
fn autostart_rejects_a_directory_owned_by_another_uid() {
	for (kind, relative) in [
		("service", "systemd/user"),
		("quadlet", "containers/systemd"),
	] {
		let w = Workspace::new();
		let foreign = std::path::Path::new("/");
		// Run as root (the Debian package build does), the workspace and `/` have the
		// same owner and there is no foreign directory to refuse: nothing to test.
		if fs::metadata(foreign).unwrap().uid() == fs::metadata(w.root.path()).unwrap().uid() {
			eprintln!("skipped: the test user owns / as well");
			return;
		}
		let dir = w.config.join(relative);
		fs::create_dir_all(dir.parent().unwrap()).unwrap();
		// A read-only fixture: root belongs to another UID and cannot be written here.
		symlink(foreign, &dir).unwrap();
		let out = w.install(kind);
		assert!(!out.status.success());
		assert!(
			stderr(&out).contains("owned by UID"),
			"{kind}: {}",
			stderr(&out)
		);
		fs::remove_file(&dir).unwrap();
		fs::create_dir(&dir).unwrap();
		chmod(&dir, 0o700);
		let out = w.install(kind);
		assert!(
			stderr(&out).contains("podup: wrote"),
			"positive control: {}",
			stderr(&out)
		);
	}
}

#[test]
fn autostart_warns_but_installs_into_a_group_writable_directory() {
	// Umask 002 makes this the ordinary mode of a home directory on Debian,
	// Ubuntu and Fedora, so refusing would break most installs.
	for kind in ["service", "quadlet"] {
		for bits in [0o775, 0o770] {
			let w = Workspace::new();
			let relative = if kind == "service" {
				"systemd/user"
			} else {
				"containers/systemd"
			};
			let dir = w.config.join(relative);
			fs::create_dir_all(&dir).unwrap();
			chmod(&dir, bits);
			let out = w.install(kind);
			let err = stderr(&out);
			assert!(err.contains("podup: wrote"), "{kind} {bits:o}: {err}");
			assert!(
				err.contains("writable by its group"),
				"{kind} {bits:o}: {err}"
			);
			assert_eq!(mode(&dir), bits);
		}
	}
}
