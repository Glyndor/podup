use super::*;

// Independent opens must contend on the same lock throughout the self-test.
#[test]
fn a_competing_update_cannot_destroy_the_rollback_backup() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"original").unwrap();
	let err = install_verified_at(
		&target,
		b"bad update",
		|_| Ok(()),
		|path| {
			let second = install_verified_at(path, b"competing update", |_| Ok(()), |_| Ok(()));
			assert!(second.is_err(), "a second transaction must be refused");
			assert_eq!(
				std::fs::read(path.with_extension("old")).unwrap(),
				b"original"
			);
			assert_eq!(std::fs::read(path).unwrap(), b"bad update");
			Err(ComposeError::Update("failed probe".into()))
		},
	)
	.unwrap_err();
	assert!(err.to_string().contains("rolled back"), "{err}");
	assert_eq!(std::fs::read(&target).unwrap(), b"original");
	// Failure must release the lock, even though the persistent sidecar remains.
	install_verified_at(&target, b"good update", |_| Ok(()), |_| Ok(())).unwrap();
	install_verified_at(&target, b"next update", |_| Ok(()), |_| Ok(())).unwrap();
	assert_eq!(std::fs::read(&target).unwrap(), b"next update");
	assert!(!target.with_extension("old").exists());
}

#[test]
fn a_staging_failure_releases_the_transaction_lock() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"original").unwrap();
	let tmp = dir
		.path()
		.join(format!(".podup.update-{}", std::process::id()));
	std::fs::create_dir(&tmp).unwrap();
	assert!(
		install_verified_at(&target, b"new", |_| Ok(()), |_| panic!("staging failed")).is_err()
	);
	assert_eq!(std::fs::read(&target).unwrap(), b"original");
	std::fs::remove_dir(tmp).unwrap();
	install_verified_at(&target, b"retry", |_| Ok(()), |_| Ok(())).unwrap();
	assert_eq!(std::fs::read(&target).unwrap(), b"retry");
}

#[test]
fn losing_the_backup_during_the_probe_reports_a_rollback_failure() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"original").unwrap();
	let err = install_verified_at(
		&target,
		b"bad update",
		|_| Ok(()),
		|path| {
			std::fs::remove_file(path.with_extension("old")).unwrap();
			Err(ComposeError::Update("failed probe".into()))
		},
	)
	.unwrap_err();
	assert!(err.to_string().contains("failed to roll back"), "{err}");
	assert!(!err.to_string().contains("rolled back"), "{err}");
	assert_eq!(std::fs::read(target).unwrap(), b"bad update");
}

/// An update decided against the version this process started as can find, under
/// the lock, that a newer one was installed in the meantime. It must not put an
/// older release over it, and it must leave the file and the backup alone.
#[test]
fn a_precheck_that_refuses_leaves_the_installed_file_alone() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"installed").unwrap();
	let err = install_verified_at(
		&target,
		b"older release",
		|_| Err(ComposeError::Update("newer is installed".into())),
		|_| panic!("the install must not go on after a refusal"),
	)
	.unwrap_err();
	assert!(err.to_string().contains("newer is installed"), "{err}");
	assert_eq!(std::fs::read(&target).unwrap(), b"installed");
	assert!(
		!target.with_extension("old").exists(),
		"no backup is made for an install that never started"
	);
	// The lock was released: a later install goes through.
	install_verified_at(&target, b"next", |_| Ok(()), |_| Ok(())).unwrap();
}

fn version(text: &str) -> crate::update::verify::Version {
	crate::update::verify::parse_version(text).unwrap()
}

#[test]
fn an_older_release_is_refused_over_a_newer_installed_one() {
	let err = ensure_not_older(Some(version("5.15.0")), "5.14.0").unwrap_err();
	let message = err.to_string();
	assert!(
		message.contains("5.15.0") && message.contains("5.14.0") && message.contains("--force"),
		"{message}"
	);
}

#[test]
fn an_equal_or_newer_release_goes_through() {
	assert!(ensure_not_older(Some(version("5.14.0")), "5.14.0").is_ok());
	assert!(ensure_not_older(Some(version("5.14.0")), "5.15.0").is_ok());
}

/// A binary that cannot say what it is gives no reason to refuse: that is also
/// the state of a broken install an update is meant to repair.
#[test]
fn an_installed_version_that_cannot_be_read_does_not_block_the_update() {
	assert!(ensure_not_older(None, "5.14.0").is_ok());
}

/// The installed version is whatever the file reports, not what this process
/// was started as.
#[cfg(unix)]
#[test]
fn the_installed_version_is_read_from_the_binary() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, "#!/bin/sh\necho 'podup 9.8.7'\n").unwrap();
	std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
	assert_eq!(installed_version(&target), Some(version("9.8.7")));
	assert_eq!(installed_version(&dir.path().join("missing")), None);
}
