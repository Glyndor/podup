#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::engine::secrets::tests_support::{engine_on, file_with_content_secrets};

/// #1219: on a first `up` every secret inspect is a 404, so there is nothing
/// to remove: the delete-then-create was spending a round trip per secret
/// deleting a secret that does not exist. Measured on the six-secret bench
/// scenario, this is what takes a cold `up` from 25 requests to 19.
#[tokio::test]
#[cfg(unix)]
async fn create_skips_the_delete_when_the_secret_does_not_exist() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/secrets/") {
			(404, r#"{"message":"no such secret"}"#.to_string())
		} else {
			(201, r#"{"ID":"abc"}"#.to_string())
		}
	});
	let e = engine_on(&fake);

	e.create_project_secrets(&file_with_content_secrets(3))
		.await
		.expect("creating fresh secrets should succeed");

	let seen = fake.requests.lock().unwrap().clone();
	let deletes: Vec<&String> = seen.iter().filter(|r| r.starts_with("DELETE")).collect();
	assert!(
		deletes.is_empty(),
		"no delete should be issued for a secret that does not exist, got {deletes:?}"
	);
	assert_eq!(
		seen.iter()
			.filter(|r| r.contains("/secrets/create"))
			.count(),
		3,
		"every secret is still created"
	);
}

/// The other half of the same rule: a secret that IS there still has to be
/// removed before the create, because `replace=true` is rejected on some
/// Podman 5.x builds. Skipping the delete unconditionally would break
/// re-`up` idempotence, which is the reason the delete exists at all.
#[tokio::test]
#[cfg(unix)]
async fn create_still_deletes_a_secret_that_already_exists() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/secrets/") {
			(
				200,
				r#"{"Spec":{"Labels":{"podup.project":"proj"}}}"#.to_string(),
			)
		} else {
			(201, r#"{"ID":"abc"}"#.to_string())
		}
	});
	let e = engine_on(&fake);

	e.create_project_secrets(&file_with_content_secrets(2))
		.await
		.expect("replacing our own secrets should succeed");

	let seen = fake.requests.lock().unwrap().clone();
	assert_eq!(
		seen.iter().filter(|r| r.starts_with("DELETE")).count(),
		2,
		"each existing secret is removed before being recreated, got {seen:?}"
	);
}

/// The behaviour change the fan-out carries, stated in #1219 and asserted
/// here rather than left to the reader: the pass no longer stops at the
/// first failure, so every secret is attempted, and the error that surfaces
/// is the first *by name*, not whichever chain happened to lose the race.
#[tokio::test]
#[cfg(unix)]
async fn fan_out_attempts_every_secret_and_reports_the_first_by_name() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/secrets/") {
			(404, r#"{"message":"no such secret"}"#.to_string())
		} else if method == "POST" && (target.contains("s2") || target.contains("s3")) {
			(500, r#"{"message":"boom"}"#.to_string())
		} else {
			(201, r#"{"ID":"abc"}"#.to_string())
		}
	});
	let e = engine_on(&fake);

	let err = e
		.create_project_secrets(&file_with_content_secrets(4))
		.await
		.expect_err("a failing secret must still fail the stage");

	let seen = fake.requests.lock().unwrap().clone();
	for i in 1..=4 {
		assert!(
			seen.iter().any(|r| r.contains(&format!("s{i}"))),
			"secret s{i} should have been attempted despite an earlier failure, got {seen:?}"
		);
	}
	assert!(
		err.to_string().contains("s2"),
		"the first failure by name should be the one reported, got: {err}"
	);
}

/// Skipping the delete opens a window between the inspect and the create.
/// If something claims the name in it, `up` fails rather than clobbering
/// what arrived, but Podman's own message for that is an opaque 500, so
/// the failure has to name the race or the operator cannot act on it.
#[tokio::test]
#[cfg(unix)]
async fn a_name_claimed_after_the_inspect_fails_with_a_legible_message() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/secrets/") {
			(404, r#"{"message":"no such secret"}"#.to_string())
		} else {
			(500, r#"{"message":"secret name in use"}"#.to_string())
		}
	});
	let e = engine_on(&fake);

	let err = e
		.create_project_secrets(&file_with_content_secrets(1))
		.await
		.expect_err("a name claimed in the window must fail")
		.to_string();

	assert!(
		err.contains("in between"),
		"the message must explain the race, got: {err}"
	);
	assert!(
		err.contains("proj_secret_s1"),
		"the message must name which secret, got: {err}"
	);
	// The engine's own error is kept as the cause, since it is useful, but it must
	// not be the whole of what the operator is handed, which is what the bare
	// `ComposeError::Podman` passthrough would have given them.
	assert!(
		!err.starts_with("podman API error"),
		"the raw engine error must not be the message itself, got: {err}"
	);
}

/// The ownership guard is untouched by any of the above: a secret of the
/// same name that podup did not create is still refused, never deleted.
#[tokio::test]
#[cfg(unix)]
async fn a_foreign_secret_is_still_refused_and_never_deleted() {
	let fake = fake_podman::start(|method, target| {
		if method == "GET" && target.contains("/secrets/") {
			(
				200,
				r#"{"Spec":{"Labels":{"podup.project":"someone-else"}}}"#.to_string(),
			)
		} else {
			(201, r#"{"ID":"abc"}"#.to_string())
		}
	});
	let e = engine_on(&fake);

	let err = e
		.create_project_secrets(&file_with_content_secrets(1))
		.await
		.expect_err("a foreign secret must not be overwritten")
		.to_string();

	assert!(err.contains("refusing to overwrite"), "got: {err}");
	let seen = fake.requests.lock().unwrap().clone();
	assert!(
		!seen.iter().any(|r| r.starts_with("DELETE")),
		"a foreign secret must never be deleted, got {seen:?}"
	);
}

/// A file secret larger than Podman accepts is refused when it is read, before
/// anything is uploaded and before the bytes are kept.
#[tokio::test]
#[cfg(unix)]
async fn an_oversized_file_secret_is_refused_before_any_upload() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("blob"), vec![b'x'; 600_000]).unwrap();
	let fake = fake_podman::start(|_, _| (201, r#"{"ID":"abc"}"#.to_string()));
	let e = crate::engine::Engine::with_base_dir(fake.client(), "proj".into(), dir.path().into());
	let file = crate::parse_str(
		"services:\n  web:\n    image: x\n    secrets: [big]\nsecrets:\n  big:\n    file: ./blob\n",
	)
	.unwrap();
	let error = e.create_project_secrets(&file).await.unwrap_err();
	assert!(error.to_string().contains("blob"), "{error}");
	assert!(
		!fake
			.requests
			.lock()
			.unwrap()
			.iter()
			.any(|r| r.contains("/secrets/create")),
		"nothing may be uploaded for a payload that is refused"
	);
}

/// Each payload under its own limit is not enough: all of them are read before
/// the first upload, so the sum is bounded too.
#[tokio::test]
#[cfg(unix)]
async fn the_secrets_of_a_project_are_bounded_together() {
	let dir = tempfile::tempdir().unwrap();
	std::fs::write(dir.path().join("blob"), vec![b'x'; 500_000]).unwrap();
	let mut yaml = String::from("services:\n  web:\n    image: x\n    secrets: [");
	let names: Vec<String> = (0..80).map(|i| format!("s{i}")).collect();
	yaml.push_str(&names.join(", "));
	yaml.push_str("]\nsecrets:\n");
	for name in &names {
		yaml.push_str(&format!("  {name}:\n    file: ./blob\n"));
	}
	let fake = fake_podman::start(|_, _| (201, r#"{"ID":"abc"}"#.to_string()));
	let e = crate::engine::Engine::with_base_dir(fake.client(), "proj".into(), dir.path().into());
	let error = e
		.create_project_secrets(&crate::parse_str(&yaml).unwrap())
		.await
		.unwrap_err();
	assert!(error.to_string().contains("together"), "{error}");
}

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

// The variables of these tests stay under 128 KiB: the kernel refuses to start a
// process when a single string of its environment is longer, and a test that
// runs a child (`mkfifo`) at the same time would fail with "Argument list too
// long". The joint limit is reached with more names instead.
#[test]
#[cfg(unix)]
fn environment_secrets_exceeding_the_shared_budget_are_refused_before_any_upload() {
	let big = "x".repeat(120_000);
	temp_env::with_var("BIG", Some(big.as_str()), || {
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let names: Vec<String> = (0..300).map(|i| format!("s{i}")).collect();
				let mut yaml = format!(
					"services:\n  web:\n    image: x\n    secrets: [{}]\nsecrets:\n",
					names.join(", ")
				);
				for name in &names {
					yaml.push_str(&format!("  {name}:\n    environment: BIG\n"));
				}
				let file = crate::parse_str(&yaml).unwrap();
				let fake = fake_podman::start(|_, _| (201, r#"{"ID":"abc"}"#.to_string()));
				let error = engine_on(&fake)
					.create_project_secrets(&file)
					.await
					.unwrap_err();
				assert!(error.to_string().contains("together"), "{error}");
				assert!(fake.requests.lock().unwrap().is_empty());
			});
	});
}

#[test]
#[cfg(unix)]
fn an_unset_environment_secret_is_refused_before_any_upload() {
	temp_env::with_var("PODUP_TEST_MISSING", None::<&str>, || {
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let file = crate::parse_str(
					"services:\n  web:\n    image: x\n    secrets: [tok]\nsecrets:\n  tok:\n    environment: PODUP_TEST_MISSING\n",
				)
				.unwrap();
				let fake = fake_podman::start(|_, _| (201, r#"{"ID":"abc"}"#.to_string()));
				let error = engine_on(&fake).create_project_secrets(&file).await.unwrap_err();
				let message = error.to_string();
				assert!(message.contains("PODUP_TEST_MISSING"), "{message}");
				assert!(message.contains("not set"), "{message}");
				assert!(fake.requests.lock().unwrap().is_empty());
			});
	});
}

#[test]
#[cfg(unix)]
fn a_shared_environment_secret_is_budgeted_and_uploaded_once() {
	let big = "x".repeat(120_000);
	temp_env::with_var("PODUP_TEST_SHARED_SECRET", Some(big.as_str()), || {
		tokio::runtime::Builder::new_current_thread()
			.enable_all()
			.build()
			.unwrap()
			.block_on(async {
				let mut yaml = String::from("services:\n");
				for i in 0..80 {
					yaml.push_str(&format!("  web{i}:\n    image: x\n    secrets: [tok]\n"));
				}
				yaml.push_str("secrets:\n  tok:\n    environment: PODUP_TEST_SHARED_SECRET\n");
				let file = crate::parse_str(&yaml).unwrap();
				let fake = fake_podman::start(|method, _| {
					if method == "GET" {
						(404, r#"{"message":"no such secret"}"#.to_string())
					} else {
						(201, r#"{"ID":"abc"}"#.to_string())
					}
				});
				engine_on(&fake)
					.create_project_secrets(&file)
					.await
					.unwrap();
				assert_eq!(
					fake.requests
						.lock()
						.unwrap()
						.iter()
						.filter(|r| r.contains("/secrets/create"))
						.count(),
					1
				);
				assert!(fake
					.bodies
					.lock()
					.unwrap()
					.iter()
					.any(|body| body == big.as_bytes()));
			});
	});
}

#[tokio::test]
#[cfg(unix)]
async fn invalid_inline_secret_sizes_are_refused_before_any_upload() {
	for content in [String::new(), "x".repeat(512_000)] {
		let mut file = file_with_content_secrets(1);
		file.secrets.get_mut("s1").unwrap().content = Some(content);
		let fake = fake_podman::start(|_, _| (201, r#"{"ID":"abc"}"#.to_string()));
		let error = engine_on(&fake)
			.create_project_secrets(&file)
			.await
			.unwrap_err();
		assert!(error.to_string().contains("bytes"), "{error}");
		assert!(fake.requests.lock().unwrap().is_empty());
	}
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
