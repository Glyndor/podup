use super::*;

// Independent opens must contend on the same lock throughout the self-test.
#[test]
fn a_competing_update_cannot_destroy_the_rollback_backup() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"original").unwrap();
	let err = install_verified_at(&target, b"bad update", |path| {
		let second = install_verified_at(path, b"competing update", |_| Ok(()));
		assert!(second.is_err(), "a second transaction must be refused");
		assert_eq!(
			std::fs::read(path.with_extension("old")).unwrap(),
			b"original"
		);
		assert_eq!(std::fs::read(path).unwrap(), b"bad update");
		Err(ComposeError::Update("failed probe".into()))
	})
	.unwrap_err();
	assert!(err.to_string().contains("rolled back"), "{err}");
	assert_eq!(std::fs::read(&target).unwrap(), b"original");
	// Failure must release the lock, even though the persistent sidecar remains.
	install_verified_at(&target, b"good update", |_| Ok(())).unwrap();
	install_verified_at(&target, b"next update", |_| Ok(())).unwrap();
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
	assert!(install_verified_at(&target, b"new", |_| panic!("staging failed")).is_err());
	assert_eq!(std::fs::read(&target).unwrap(), b"original");
	std::fs::remove_dir(tmp).unwrap();
	install_verified_at(&target, b"retry", |_| Ok(())).unwrap();
	assert_eq!(std::fs::read(&target).unwrap(), b"retry");
}

#[test]
fn losing_the_backup_during_the_probe_reports_a_rollback_failure() {
	let dir = tempfile::tempdir().unwrap();
	let target = dir.path().join("podup");
	std::fs::write(&target, b"original").unwrap();
	let err = install_verified_at(&target, b"bad update", |path| {
		std::fs::remove_file(path.with_extension("old")).unwrap();
		Err(ComposeError::Update("failed probe".into()))
	})
	.unwrap_err();
	assert!(err.to_string().contains("failed to roll back"), "{err}");
	assert!(!err.to_string().contains("rolled back"), "{err}");
	assert_eq!(std::fs::read(target).unwrap(), b"bad update");
}
