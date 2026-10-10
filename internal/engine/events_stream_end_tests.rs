use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::{Engine, EventsOptions};

fn engine(fake: &fake_podman::FakePodman) -> Engine {
	Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir())
}

/// One event, then the body ends the way the server chose.
fn fake(reply: fn() -> FakeReply) -> fake_podman::FakePodman {
	fake_podman::start_replying(move |_method, _target| reply())
}

fn one_event() -> Vec<String> {
	vec![r#"{"Type":"container","Action":"start","id":"abc"}"#.to_string()]
}

/// Both ends of an elapsed window, which is the only form libpod closes.
fn bounded() -> EventsOptions {
	EventsOptions {
		since: Some("2026-01-01T00:00:00Z".to_string()),
		until: Some("2026-01-01T01:00:00Z".to_string()),
		..Default::default()
	}
}

#[tokio::test]
async fn an_unbounded_feed_that_ends_cleanly_is_still_a_failure() {
	// The case no error-shaped check could catch: the server closed the body
	// properly, so the parser reports a clean end, and this used to exit 0.
	let fake = fake(|| FakeReply::ChunkedEnd(one_event()));
	let err = engine(&fake)
		.stream_events_with_options(false, &EventsOptions::default())
		.await
		.expect_err("only the client ends an unbounded feed, so any end is unexpected");
	assert!(
		matches!(err, crate::error::ComposeError::StreamTruncated(_)),
		"expected the intent verdict, got {err:?}"
	);
}

#[tokio::test]
async fn an_unbounded_feed_cut_mid_body_is_a_failure() {
	let fake = fake(|| FakeReply::ChunkedTruncated(one_event()));
	let err = engine(&fake)
		.stream_events_with_options(false, &EventsOptions::default())
		.await
		.expect_err("a severed unbounded feed is a failure too");
	assert!(
		matches!(err, crate::error::ComposeError::Podman(_)),
		"the transport error must survive so the operator sees the cause, got {err:?}"
	);
}

#[tokio::test]
async fn a_bounded_feed_that_ends_is_success() {
	// `--until` is the client saying the window closes on its own, so the end
	// is what was asked for. Measured on 5.4.2: an already-elapsed window does
	// end the feed cleanly.
	let fake = fake(|| FakeReply::ChunkedEnd(one_event()));
	engine(&fake)
		.stream_events_with_options(false, &bounded())
		.await
		.expect("a bounded feed reaching the end of its window succeeded");
}

/// `--until` alone does not bound anything: measured against 5.4.2, libpod
/// leaves the feed open without a `since` to pair it with. Treating it as
/// bounded would call an unbounded feed bounded and hand back a success.
#[tokio::test]
async fn until_without_since_is_not_a_bounded_feed() {
	let fake = fake(|| FakeReply::ChunkedEnd(one_event()));
	let opts = EventsOptions {
		until: Some("2026-01-01T00:00:00Z".to_string()),
		..Default::default()
	};
	let err = engine(&fake)
		.stream_events_with_options(false, &opts)
		.await
		.expect_err("until alone leaves the feed unbounded, so any end is unexpected");
	assert!(
		matches!(err, crate::error::ComposeError::StreamTruncated(_)),
		"expected the unbounded verdict, got {err:?}"
	);
}

#[tokio::test]
async fn a_bounded_feed_cut_mid_body_is_a_failure() {
	// Intent says whether an *ending* was expected. It cannot make a severed
	// socket expected, and this is the case that matters most: `--until` with
	// `--format json` is the scriptable form, so swallowing the error here
	// would truncate a window and report success on exactly the path a script
	// trusts, while the interactive unbounded form kept the strict check.
	// That inverts #1104 rather than completing it.
	let fake = fake(|| FakeReply::ChunkedTruncated(one_event()));
	let err = engine(&fake)
		.stream_events_with_options(false, &bounded())
		.await
		.expect_err("a severed window is a failed read, bounded or not");
	assert!(
		matches!(err, crate::error::ComposeError::Podman(_)),
		"the transport error must survive so the operator sees the cause, got {err:?}"
	);
}

/// `#2014`: an unknown service name has to fail with
/// `ComposeError::ServiceNotFound` BEFORE the events stream is opened.
/// `docker compose events nope` errors the same way; failing later would
/// mean a script that passed `--filter` past the typo silently watched the
/// full feed. The fake's request log is the only witness for "no HTTP
/// went out": a missing name has to be caught before any other request.
#[tokio::test]
async fn an_unknown_service_fails_before_any_request() {
	use crate::compose::types::{ComposeFile, Service};

	let fake = fake(|| {
		// The body is irrelevant; the call must reject before the
		// engine ever asks for it. The matching FakeReply keeps the test
		// honest if the validation gate ever moves below the request: a
		// body that asks for a 200 here means a successful request would
		// not have raised.
		FakeReply::ChunkedEnd(one_event())
	});

	let mut file = ComposeFile::default();
	file.services.insert("web".to_string(), Service::default());

	let opts = EventsOptions::new(None, None, vec![]).with_services(vec!["nope".to_string()]);
	let err = engine(&fake)
		.stream_service_events(&file, false, &opts)
		.await
		.expect_err(
			"`events nope` on a project that has only `web` must fail with ServiceNotFound",
		);

	let msg = format!("{err}");
	assert_eq!(
		msg, "service 'nope' not found",
		"the wording is fixed by `ServiceNotFound`'s Display impl and shared with every other command (#2014); got {msg}"
	);
	assert!(
		fake.requests.lock().unwrap().is_empty(),
		"validation must run before any HTTP goes out: requests={:?}",
		fake.requests.lock().unwrap()
	);
}
