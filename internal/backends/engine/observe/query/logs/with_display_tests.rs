#[cfg(unix)]
use crate::compose::types::{ComposeFile, Service};
#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::engine::query::LogsOptions;
#[cfg(unix)]
use crate::engine::Engine;

#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

/// After a runtime `scale web=3`, Podman has three `proj-web-*` containers
/// while the compose file still declares a single (unscaled) replica.
/// `logs web` must target every live container, not just the one the
/// static replica count predicts.
#[cfg(unix)]
#[tokio::test]
async fn logs_targets_every_live_replica_after_scale() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-3"],"Labels":{"podup.service":"web"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if method == "GET" && target.contains("/logs") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	e.logs_with_options(&file, &["web".to_string()], LogsOptions::default())
		.await
		.expect("logs should succeed");

	let seen = fake.requests.lock().unwrap();
	for i in 1..=3 {
		assert!(
			seen.iter()
				.any(|r| r.contains(&format!("/proj-web-{i}/logs"))),
			"expected proj-web-{i} to be targeted after scale: {seen:?}"
		);
	}
}

/// Resolving replicas for one selected service must not abort `logs` before
/// a single line prints for the others: `logs` already documents that it
/// tolerates a missing/not-yet-created container this way (see the
/// per-container `get_stream` handling in `logs_with_display`).
///
/// #1445 collapsed the per-service `live_replica_names` resolution into a
/// single bulk fetch (`live_project_replicas_sorted`), so the per-service
/// error path this test used to drive no longer exists: there is one
/// container-list call for the whole project, not one per service, so a
/// per-service 500 is no longer reachable in this code. The companion
/// `logs_fails_when_no_service_resolves_at_all` covers the only failure
/// mode that survives the bulk refactor: the bulk GET errors out, no
/// target survives, and `logs` exits non-zero instead of printing nothing
/// and reporting success.
/// The other side of the tolerance: when *nothing* resolves, there is no
/// partial result left to protect.
///
/// An unreachable engine made `logs` print nothing and exit 0, which is
/// indistinguishable from a project that simply has no logs, so a health check
/// or deploy gate built on `compose logs` read success from an engine that was
/// not there. Every service 500s here, so no target survives.
///
/// This is not #1104: nothing classifies how a stream *ended*. The requests
/// never opened.
#[cfg(unix)]
#[tokio::test]
async fn logs_fails_when_no_service_resolves_at_all() {
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(500, r#"{"message":"internal server error"}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("a".into(), Service::default());
	file.services.insert("b".into(), Service::default());

	let out = e
		.logs_with_options(
			&file,
			&["a".to_string(), "b".to_string()],
			LogsOptions::default(),
		)
		.await;
	assert!(
		out.is_err(),
		"logs must not report success when it could reach nothing at all"
	);
}

/// And one container failing to stream while another succeeds stays tolerated,
/// so the logs that exist are still shown.
#[cfg(unix)]
#[tokio::test]
async fn logs_tolerates_one_container_that_will_not_stream() {
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			if target.contains("podup.service%3Da") {
				(200, r#"[{"Names":["/proj-a-1"]}]"#.to_string())
			} else {
				(200, r#"[{"Names":["/proj-b-1"]}]"#.to_string())
			}
		} else if method == "GET" && target.contains("/proj-a-1/logs") {
			(500, r#"{"message":"boom"}"#.to_string())
		} else if method == "GET" && target.contains("/logs") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("a".into(), Service::default());
	file.services.insert("b".into(), Service::default());

	e.logs_with_options(
		&file,
		&["a".to_string(), "b".to_string()],
		LogsOptions::default(),
	)
	.await
	.expect("one container failing to stream must not blank the other");
}

/// A `logs -f` over two containers must not report success when one of them
/// lost its stream while its container kept running.
///
/// The transport cannot tell an open failure from a stream that died
/// mid-output, so the engine re-checks the container out of band: still
/// running means live output was truncated (#1104). What it must not do is
/// let a sibling that finished cleanly cover that loss. The operator read
/// exit 0 and never learned that a container's output was cut.
#[cfg(unix)]
#[tokio::test]
async fn logs_follow_reports_a_truncated_stream_even_when_a_sibling_finished_cleanly() {
	// One multiplexed stdout frame, the shape libpod frames its log body in.
	let frame = "\u{1}\0\0\0\0\0\0\u{5}hello".to_string();
	let containers = r#"[
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"},"State":"running"},
		{"Names":["/proj-worker-1"],"Labels":{"podup.service":"worker"},"State":"running"}
	]"#;
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			fake_podman::FakeReply::Body(200, containers.to_string())
		} else if method == "GET" && target.contains("/proj-web-1/logs") {
			// The stream dies where the chunk terminator should be, and the
			// container keeps running: live output was truncated.
			fake_podman::FakeReply::ChunkedTruncated(vec![frame.clone()])
		} else if method == "GET" && target.contains("/proj-worker-1/logs") {
			// The sibling ends the way it should.
			fake_podman::FakeReply::ChunkedEnd(vec![frame.clone()])
		} else {
			fake_podman::FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());
	file.services.insert("worker".into(), Service::default());

	let out = e
		.logs_with_options(
			&file,
			&["web".to_string(), "worker".to_string()],
			LogsOptions::new(true, None, None, None, false),
		)
		.await;
	assert!(
		out.is_err(),
		"a truncated stream must not be covered by a sibling that finished cleanly"
	);
}

/// The other side of the same rule, and the reason the truncation is
/// reported at all: a container that stopped takes its stream down with it,
/// which is a finished read and not a lost one.
#[cfg(unix)]
#[tokio::test]
async fn logs_follow_accepts_a_stream_that_ended_with_its_container() {
	let frame = "\u{1}\0\0\0\0\0\0\u{5}hello".to_string();
	let containers = r#"[
		{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"},"State":"exited"},
		{"Names":["/proj-worker-1"],"Labels":{"podup.service":"worker"},"State":"running"}
	]"#;
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			fake_podman::FakeReply::Body(200, containers.to_string())
		} else if method == "GET" && target.contains("/proj-web-1/logs") {
			fake_podman::FakeReply::ChunkedTruncated(vec![frame.clone()])
		} else if method == "GET" && target.contains("/proj-worker-1/logs") {
			fake_podman::FakeReply::ChunkedEnd(vec![frame.clone()])
		} else {
			fake_podman::FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());
	file.services.insert("worker".into(), Service::default());

	e.logs_with_options(
		&file,
		&["web".to_string(), "worker".to_string()],
		LogsOptions::new(true, None, None, None, false),
	)
	.await
	.expect("a stream that ended with its container is a finished read");
}

/// The history of a stopped container, read to its end, must not report
/// success when the frames the daemon sent do not add up.
///
/// A frame that was cut short is a failed read whatever the container was
/// doing: the corruption is in the data, not in the timing. The container's
/// state used to decide it, and a stopped container's state said "the stream
/// ended because the container stopped", so the command reported a complete
/// history it had not received.
#[cfg(unix)]
#[tokio::test]
async fn logs_report_a_truncated_frame_from_a_stopped_container() {
	// A frame header that promises five bytes and delivers two.
	let frame = "\u{1}\0\0\0\0\0\0\u{5}he".to_string();
	let containers =
		r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"},"State":"exited"}]"#;
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			fake_podman::FakeReply::Body(200, containers.to_string())
		} else if method == "GET" && target.contains("/proj-web-1/logs") {
			// The body ends where it should, but the last frame inside it
			// was cut short.
			fake_podman::FakeReply::ChunkedEnd(vec![frame.clone()])
		} else {
			fake_podman::FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	let out = e
		.logs_with_options(&file, &["web".to_string()], LogsOptions::default())
		.await;
	assert!(
		out.is_err(),
		"a history whose frames do not add up must not be reported as read"
	);
}

/// The same rule for a frame the daemon sent over the limit: the read failed,
/// so the command failed.
#[cfg(unix)]
#[tokio::test]
async fn logs_report_an_oversized_frame_from_a_stopped_container() {
	// A frame header announcing four gigabytes, which is over the limit the
	// client accepts.
	let frame = "\u{1}\0\0\0\u{ff}\u{ff}\u{ff}\u{ff}".to_string();
	let containers =
		r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"},"State":"exited"}]"#;
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			fake_podman::FakeReply::Body(200, containers.to_string())
		} else if method == "GET" && target.contains("/proj-web-1/logs") {
			fake_podman::FakeReply::ChunkedEnd(vec![frame.clone()])
		} else {
			fake_podman::FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	let out = e
		.logs_with_options(&file, &["web".to_string()], LogsOptions::default())
		.await;
	assert!(
		out.is_err(),
		"a history with a frame over the limit must not be reported as read"
	);
}
