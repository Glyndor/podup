//! A project created from another directory is not torn down from this one.
//!
//! Two directories can resolve to the same project name (two checkouts called
//! `web`, or `COMPOSE_PROJECT_NAME` in another repository's `.env`) and then
//! share every container, network and volume. The project's networks record
//! the directory that created them, and `down -v` or `down --remove-orphans`
//! from another one would remove what that directory owns, so they are refused
//! without `--force`, before a single DELETE reaches the engine.

#![cfg(unix)]

#[path = "../support/fake_engine.rs"]
mod fake_engine;

use fake_engine::{FakeEngine, Reply};

/// The project's network says it was created from another directory.
fn created_elsewhere(line: &str, _: &str) -> Option<Reply> {
	(line.starts_with("GET") && line.contains("/networks/json")).then(|| Reply {
		status: 200,
		body: r#"[{"name":"vzh-elsewhere_default","labels":{"podup.project":"vzh-elsewhere","podup.working_dir":"/srv/the-other-checkout"}}]"#
			.to_string(),
	})
}

fn deletes(fake: &FakeEngine) -> usize {
	fake.requests()
		.iter()
		.filter(|r| r.starts_with(b"DELETE"))
		.count()
}

fn engine() -> FakeEngine {
	let fake = FakeEngine::start(created_elsewhere);
	std::fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: nginx\nvolumes:\n  data: {}\n",
	)
	.unwrap();
	fake
}

#[test]
fn down_with_volumes_from_another_directory_is_refused() {
	for flag in ["-v", "--remove-orphans"] {
		let fake = engine();
		let (code, text) = fake.run_with_status(None, &[], &["-p", "vzh-elsewhere", "down", flag]);
		assert_ne!(code, Some(0), "{flag} was not refused:\n{text}");
		assert!(
			text.contains("/srv/the-other-checkout") && text.contains("--force"),
			"{flag}: the refusal does not name the other directory and the way out:\n{text}"
		);
		assert_eq!(
			deletes(&fake),
			0,
			"{flag}: something was removed before the refusal"
		);
	}
}

#[test]
fn force_lets_the_teardown_run() {
	let fake = engine();
	let (code, text) =
		fake.run_with_status(None, &[], &["-p", "vzh-elsewhere", "down", "-v", "--force"]);
	assert!(
		!text.contains("--force to do it anyway"),
		"--force did not lift the refusal:\n{text}"
	);
	// The teardown itself runs against an engine that answers every removal.
	assert_eq!(code, Some(0), "{text}");
}
