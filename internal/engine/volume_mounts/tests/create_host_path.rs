use super::*;

#[test]
fn create_host_path_creates_directory() {
	let dir = tempfile::tempdir().unwrap();
	let rel = "subdir/nested";
	let svc = svc_with_volumes(vec![VolumeMount::Long {
		volume_type: VolumeType::Bind,
		source: Some(rel.into()),
		target: "/cont".into(),
		read_only: None,
		bind: Some(BindOptions {
			propagation: None,
			create_host_path: Some(true),
			selinux: None,
		}),
		volume: None,
		tmpfs: None,
		consistency: None,
	}]);
	build_mounts_all(&svc, dir.path());
	assert!(dir.path().join(rel).exists());
}

#[test]
fn short_form_bind_creates_missing_host_path() {
	// A short-form bind whose relative source is missing must have its host
	// directory created (compose-spec implies create_host_path for short
	// syntax), anchored to the project base dir, not left to fail with a raw
	// podman statfs 500.
	let dir = tempfile::tempdir().unwrap();
	let rel = "missing-dir";
	let svc = svc_with_volumes(vec![VolumeMount::Short(format!("./{rel}:/app/data"))]);
	let (mounts, named) = build_mounts_all(&svc, dir.path());
	assert!(named.is_empty());
	assert_eq!(mounts.len(), 1);
	assert_eq!(mounts[0].mount_type, "bind");
	assert!(
		dir.path().join(rel).is_dir(),
		"short-form bind source directory should be created"
	);
}
