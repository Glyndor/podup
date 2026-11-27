//! A replacement that fails leaves the project with the secret it had.
//! The contents stand for passwords, so no failure message holds one.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::*;

/// How the fake answers: what the old secret holds, as the API writes it, and
/// the status of each `secrets/create` in turn (the last one repeats).
struct Case {
	held: &'static str,
	creates: &'static [u16],
}

async fn replace(case: Case) -> (Option<String>, fake_podman::FakePodman) {
	let posted = Arc::new(AtomicUsize::new(0));
	let fake = fake_podman::start(move |method, _| {
		if method == "GET" {
			(
				200,
				format!(
					r#"{{"ID":"old","Spec":{{"Labels":{{"podup.project":"proj","kept":"yes"}}}},"SecretData":"{}"}}"#,
					case.held
				),
			)
		} else if method == "POST" {
			let turn = posted.fetch_add(1, Ordering::SeqCst);
			let status = case.creates[turn.min(case.creates.len() - 1)];
			(status, format!(r#"{{"ID":"new","message":"turn {turn}"}}"#))
		} else {
			(200, "{}".to_string())
		}
	});
	let e = engine_on(&fake);
	let err = e
		.create_project_secrets(&file_with_content_secrets(1))
		.await
		.err()
		.map(|e| e.to_string());
	(err, fake)
}

/// The requests that change something, and the body of each create.
fn changes(fake: &fake_podman::FakePodman) -> (Vec<String>, Vec<(String, Vec<u8>)>) {
	let requests = fake.requests.lock().unwrap().clone();
	let bodies = fake.bodies.lock().unwrap().clone();
	let changing = requests
		.iter()
		.filter(|r| !r.starts_with("GET"))
		.cloned()
		.collect();
	let creates = requests
		.into_iter()
		.zip(bodies)
		.filter(|(r, _)| r.contains("/secrets/create"))
		.collect();
	(changing, creates)
}

#[tokio::test]
async fn a_secret_that_holds_the_same_content_is_left_alone() {
	let (err, fake) = replace(Case {
		held: "v1",
		creates: &[500],
	})
	.await;
	assert_eq!(err, None);
	assert_eq!(changes(&fake).0, Vec::<String>::new());
}

#[tokio::test]
async fn a_failed_create_puts_the_previous_content_back() {
	let (err, fake) = replace(Case {
		held: "old-content",
		creates: &[500, 201],
	})
	.await;
	assert!(
		err.is_some_and(|e| e.contains("turn 0")),
		"the error of the create is what up reports"
	);
	let (changing, creates) = changes(&fake);
	assert_eq!(changing.len(), 3, "{changing:?}");
	assert!(changing[0].starts_with("DELETE") && changing[0].ends_with("/secrets/old"));
	assert_eq!(creates.len(), 2);
	assert!(
		creates[0].1 == b"v1",
		"the first create is not the new content"
	);
	assert!(
		creates[1].1 == b"old-content",
		"the second create is not the previous content"
	);
	assert!(
		creates[1].0.contains("kept") && creates[1].0.contains("proj_secret_s1"),
		"the previous name and labels are not restored"
	);
	assert!(
		creates.iter().all(|(r, _)| !r.contains("replace")),
		"a create must never overwrite a name"
	);
}

#[tokio::test]
async fn a_replacement_that_works_creates_once() {
	let (err, fake) = replace(Case {
		held: "old-content",
		creates: &[201],
	})
	.await;
	assert_eq!(err, None);
	let (_, creates) = changes(&fake);
	assert_eq!(creates.len(), 1);
	assert!(creates[0].1 == b"v1", "the create is not the new content");
}

/// Content the API cannot give back as it is must not be put back altered.
#[tokio::test]
async fn content_that_is_not_text_is_not_put_back() {
	let (err, fake) = replace(Case {
		held: "\\ufffd",
		creates: &[500, 201],
	})
	.await;
	assert!(err.is_some_and(|e| e.contains("turn 0")));
	assert_eq!(changes(&fake).1.len(), 1);
}

#[tokio::test]
async fn a_restore_that_fails_still_reports_the_first_error() {
	let (err, fake) = replace(Case {
		held: "old-content",
		creates: &[500],
	})
	.await;
	assert!(err.is_some_and(|e| e.contains("turn 0")));
	assert_eq!(changes(&fake).1.len(), 2);
}
