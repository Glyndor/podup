use super::*;

// --- the size limits ------------------------------------------------------

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
