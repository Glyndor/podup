use crate::engine::fake_podman;
use crate::engine::Engine;

async fn up_error(yaml: &str, profiles: &[String]) -> (Option<crate::error::ComposeError>, usize) {
	let fake = fake_podman::start(|_, _| (200, "[]".to_string()));
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	let file = crate::parse_str(yaml).unwrap();
	let result = engine
		.up_with_options(&file, false, profiles, &[], false, false, false, false)
		.await;
	let creates = fake
		.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.starts_with("POST"))
		.count();
	(result.err(), creates)
}

/// An `up` with nothing to start is a usage error, as in docker compose, and it
/// creates nothing on the way to saying so.
#[tokio::test]
async fn up_refuses_a_file_with_no_services() {
	let (err, creates) = up_error("services: {}\n", &[]).await;
	assert!(
		matches!(err, Some(crate::error::ComposeError::Invalid(ref m)) if m.contains("no service selected")),
		"got {err:?}"
	);
	assert_eq!(creates, 0);
}

/// The same when every service sits behind a profile that is not active.
#[tokio::test]
async fn up_refuses_when_every_service_is_behind_an_inactive_profile() {
	let yaml = "services:\n  a:\n    image: alpine\n    profiles: [p]\n";
	let (err, creates) = up_error(yaml, &[]).await;
	assert!(
		matches!(err, Some(crate::error::ComposeError::Invalid(_))),
		"got {err:?}"
	);
	assert_eq!(creates, 0);
	// Control: activating the profile selects the service, so no refusal.
	let (err, _) = up_error(yaml, &["p".to_string()]).await;
	assert!(
		!matches!(err, Some(crate::error::ComposeError::Invalid(ref m)) if m.contains("no service selected")),
		"got {err:?}"
	);
}
