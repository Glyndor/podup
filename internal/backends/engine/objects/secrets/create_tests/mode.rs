use super::*;

// --- the mode a file secret is mounted with -------------------------------

/// The mode of a file secret and its bytes come from the same read. If another
/// file is put under the name between the upload and the mount, the container
/// still gets the mode of the file whose bytes were uploaded, not the new one's.
#[tokio::test]
#[cfg(unix)]
async fn the_mode_of_a_file_secret_is_the_one_of_the_file_that_was_read() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("credential");
	std::fs::write(&path, b"private").unwrap();
	std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/secrets/") {
			(404, r#"{"message":"no such secret"}"#.to_string())
		} else {
			(201, r#"{"ID":"abc"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::with_base_dir(fake.client(), "proj".into(), dir.path().into());
	let file = crate::parse_str(
		"services:\n  web:\n    image: x\n    secrets: [cred]\nsecrets:\n  cred:\n    file: ./credential\n",
	)
	.unwrap();
	e.create_project_secrets(&file).await.unwrap();

	// A rotation puts a different, world-readable file under the same name.
	let replacement = dir.path().join("replacement");
	std::fs::write(&replacement, b"other").unwrap();
	std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o644)).unwrap();
	std::fs::rename(&replacement, &path).unwrap();

	let secrets = e
		.build_native_secrets(&file.services["web"], &file)
		.await
		.unwrap();
	assert_eq!(secrets.len(), 1);
	assert_eq!(
		secrets[0].mode,
		Some(0o600),
		"the mount took the mode of the file now under the name"
	);
}

/// A file secret that was not read in this run has no mode to mount with, and
/// a guess from the path could widen it.
#[tokio::test]
#[cfg(unix)]
async fn a_file_secret_that_was_not_read_has_no_mode_to_mount_with() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("credential"), b"private").unwrap();
	let fake = fake_podman::start(|_, _| (201, r#"{"ID":"abc"}"#.to_string()));
	let e = crate::engine::Engine::with_base_dir(fake.client(), "proj".into(), dir.path().into());
	let file = crate::parse_str(
		"services:\n  web:\n    image: x\n    secrets: [cred]\nsecrets:\n  cred:\n    file: ./credential\n",
	)
	.unwrap();
	let Err(error) = e.build_native_secrets(&file.services["web"], &file).await else {
		panic!("a secret that was not read must not get a mode");
	};
	assert!(error.to_string().contains("credential"), "{error}");
}

/// The mode a file secret is mounted with is part of what decides whether the
/// container is recreated. Two runs over the same bytes under a narrower mode
/// must not hash alike, or the container keeps mounting the secret as it was.
#[tokio::test]
#[cfg(unix)]
async fn the_same_bytes_under_a_narrower_mode_change_the_hash_of_the_container() {
	use std::os::unix::fs::PermissionsExt;
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("credential");
	std::fs::write(&path, b"private").unwrap();
	std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/secrets/") {
			(404, r#"{"message":"no such secret"}"#.to_string())
		} else {
			(201, r#"{"ID":"abc"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::with_base_dir(fake.client(), "proj".into(), dir.path().into());
	let file = crate::parse_str(
		"services:\n  web:\n    image: x\n    secrets: [cred]\nsecrets:\n  cred:\n    file: ./credential\n",
	)
	.unwrap();
	let hash = |e: &crate::engine::Engine| {
		crate::engine::container::config_hash(
			&file.services["web"],
			&file,
			"proj",
			dir.path(),
			&e.uploaded_file_digests.lock().unwrap(),
			&Default::default(),
		)
		.unwrap()
	};

	e.create_project_secrets(&file).await.unwrap();
	let wide = hash(&e);
	std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
	e.create_project_secrets(&file).await.unwrap();
	let narrow = hash(&e);
	assert_ne!(wide, narrow, "only the mode changed, and the hash did not");

	// The hash a command computes without uploading anything, from the file as it
	// is, is the one the upload records.
	let without_upload = crate::engine::container::config_hash(
		&file.services["web"],
		&file,
		"proj",
		dir.path(),
		&Default::default(),
		&Default::default(),
	)
	.unwrap();
	assert_eq!(without_upload, narrow);
}
