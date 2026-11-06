use super::*;

// --- environment sources --------------------------------------------------

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
