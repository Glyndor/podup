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

fn long_bind(rel: &str, bind: Option<BindOptions>) -> VolumeMount {
	VolumeMount::Long {
		volume_type: VolumeType::Bind,
		source: Some(rel.into()),
		target: "/cont".into(),
		read_only: None,
		bind,
		volume: None,
		tmpfs: None,
		consistency: None,
	}
}

fn bind_options(create_host_path: Option<bool>) -> Option<BindOptions> {
	Some(BindOptions {
		propagation: None,
		create_host_path,
		selinux: None,
	})
}

#[test]
fn long_form_bind_without_a_bind_block_creates_the_missing_host_path() {
	// Compose Specification, services > volumes > long syntax: `create_host_path`
	// "Defaults to `true`". Docker Compose creates the directory; a long-form bind
	// that says nothing about it used to fail at the daemon with `statfs`.
	let dir = tempfile::tempdir().unwrap();
	let svc = svc_with_volumes(vec![long_bind("long-default", None)]);
	build_mounts_all(&svc, dir.path());
	assert!(dir.path().join("long-default").is_dir());
}

#[test]
fn long_form_bind_with_other_options_still_creates_the_missing_host_path() {
	let dir = tempfile::tempdir().unwrap();
	let svc = svc_with_volumes(vec![long_bind("with-options", bind_options(None))]);
	build_mounts_all(&svc, dir.path());
	assert!(dir.path().join("with-options").is_dir());
}

#[test]
fn an_explicit_false_leaves_the_missing_host_path_alone() {
	// The control: `create_host_path: false` is the way the specification gives to
	// ask for the daemon's error instead of a new directory.
	let dir = tempfile::tempdir().unwrap();
	let svc = svc_with_volumes(vec![long_bind("never", bind_options(Some(false)))]);
	let (mounts, _) = build_mounts_all(&svc, dir.path());
	assert_eq!(mounts.len(), 1);
	assert!(!dir.path().join("never").exists());
}

#[test]
fn a_long_form_bind_on_an_existing_directory_is_left_as_it_is() {
	let dir = tempfile::tempdir().unwrap();
	let existing = dir.path().join("existing");
	std::fs::create_dir(&existing).unwrap();
	std::fs::write(existing.join("keep"), "x").unwrap();
	let svc = svc_with_volumes(vec![long_bind("existing", None)]);
	build_mounts_all(&svc, dir.path());
	assert!(existing.join("keep").exists());
}

#[test]
fn a_required_bind_source_that_is_missing_fails_naming_service_and_path() {
	let dir = tempfile::tempdir().unwrap();
	let svc = svc_with_volumes(vec![long_bind("absent", bind_options(Some(false)))]);
	let err = ensure_required_bind_sources("web", &svc, dir.path())
		.expect_err("a missing source with create_host_path false must fail");
	let msg = err.to_string();
	let expected = dir.path().join("absent");
	assert!(msg.contains("\"web\""), "service not named: {msg}");
	assert!(
		msg.contains(&expected.to_string_lossy().into_owned()),
		"path not named: {msg}"
	);
	assert!(!expected.exists(), "the directory must not be created");
}

#[test]
fn a_required_bind_source_that_exists_passes() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::create_dir(dir.path().join("here")).unwrap();
	let svc = svc_with_volumes(vec![long_bind("here", bind_options(Some(false)))]);
	assert!(ensure_required_bind_sources("web", &svc, dir.path()).is_ok());
}

#[test]
fn a_bind_source_that_may_be_created_is_not_required_to_exist() {
	let dir = tempfile::tempdir().unwrap();
	for opts in [None, bind_options(None), bind_options(Some(true))] {
		let svc = svc_with_volumes(vec![long_bind("later", opts)]);
		assert!(ensure_required_bind_sources("web", &svc, dir.path()).is_ok());
	}
}
