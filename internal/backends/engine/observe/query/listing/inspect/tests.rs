use super::attach_log_query;
use crate::compose::parse_str;
// The fake Podman speaks over a unix socket and exists on unix only, like the
// test below that drives it.
#[cfg(unix)]
use crate::compose::types::{ComposeFile, Service};
#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::engine::Engine;
#[cfg(unix)]
use crate::error::ComposeError;

#[test]
fn attach_query_suppresses_log_backlog() {
	// `tail=0` means attach streams live output only, not the full history.
	let q = attach_log_query();
	assert!(q.contains("follow=true"), "got: {q}");
	assert!(q.contains("tail=0"), "got: {q}");
}

#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

/// A service with no host binding for the requested port must surface as
/// `<service> publishes no host port for <port>/<proto>`, not as an
/// `unsupported feature:` (which reads as a podup limitation). The variant
/// must not be `Unsupported` either, and the message must not carry the old
/// `no host binding for ... port ...` wording.
#[tokio::test]
#[cfg(unix)]
async fn port_without_binding_reports_publishes_no_host_port() {
	let fake = fake_podman::start(|method, target| {
		// Container list with one running container for our service.
		if method == "GET" && target.contains("/containers/json") {
			(
				200,
				r#"[{"Id":"abc123","Names":["/talker-test-talker-1"],"Image":"alpine","State":"running","Labels":{"podup.project":"talker-test","podup.service":"talker"},"Ports":[],"Created":"2026-01-01T00:00:00Z"}]"#.to_string(),
			)
		// Container inspect with NO binding for 80/tcp.
		} else if method == "GET" && target.contains("/containers/talker-test-talker-1/json") {
			(
				200,
				r#"{"State":{"Status":"running"},"NetworkSettings":{"Ports":{}}}"#.to_string(),
			)
		} else {
			(404, r#"{"message":"not used"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "talker-test");
	let file = parse_str(
		"services:\n  talker:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	let err = e
		.port_with_index(&file, "talker", "80", "tcp", None)
		.await
		.expect_err("a service with no host binding must error");
	let msg = err.to_string();
	assert_eq!(
		msg, "talker publishes no host port for 80/tcp",
		"the new message names the service and port: {msg}"
	);
	assert!(
		matches!(err, ComposeError::PortNotPublished(_)),
		"the message has its own variant, not Unsupported: {err:?}"
	);
}

/// #1742: `podup top` over a multi-service project used to issue one
/// `/containers/json` GET per service to find running replicas, mirroring the
/// pre-#1445 `logs` path. The fix collapses that into the same bulk
/// `live_project_running_replicas_sorted` lookup `logs`/`port`/`exec`/`cp`
/// already share (#1445). A regression that put the call back inside the
/// per-service loop would grow `fake.requests` by one row per service;
/// counting requests is what catches it.
#[tokio::test]
#[cfg(unix)]
async fn top_issues_one_container_list_for_the_whole_project() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-api-1"],"State":"running","Labels":{"podup.service":"api"}},
		{"Names":["/proj-db-1"],"State":"running","Labels":{"podup.service":"db"}},
		{"Names":["/proj-cache-1"],"State":"running","Labels":{"podup.service":"cache"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else if method == "GET" && target.contains("/top") {
			// The per-container `/top` payload is empty here: the test is
			// about how many listing calls are made, not what the
			// process table says.
			(200, r#"{"Titles":[],"Processes":[]}"#.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");

	let mut file = ComposeFile::default();
	for svc in ["web", "api", "db", "cache"] {
		file.services.insert(svc.into(), Service::default());
	}

	e.top_with_options(&file, &[], false)
		.await
		.expect("top over a present container list must succeed");

	let lists = fake
		.requests
		.lock()
		.unwrap()
		.iter()
		.filter(|r| r.contains("/containers/json"))
		.count();
	assert_eq!(
		lists,
		1,
		"four services must share one container-list round-trip, not one each (#1742); \
		 requests were {:?}",
		fake.requests.lock().unwrap()
	);
}

/// `attach` streams a running container's output until the container stops. A
/// stream that dies while the container keeps running lost output, and the
/// command has to say so: it used to break out of the loop and report
/// success, so a script could not tell a finished attach from a lost one.
#[tokio::test]
#[cfg(unix)]
async fn attach_reports_a_stream_that_died_under_a_running_container() {
	// One multiplexed stdout frame, the shape libpod frames its log body in.
	let frame = "\u{1}\0\0\0\0\0\0\u{5}hello".to_string();
	let containers =
		r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#;
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			fake_podman::FakeReply::Body(200, containers.to_string())
		} else if method == "GET" && target.contains("/proj-web-1/json") {
			fake_podman::FakeReply::Body(200, r#"{"State":{"Status":"running"}}"#.to_string())
		} else if method == "GET" && target.contains("/proj-web-1/logs") {
			// The stream dies where the chunk terminator should be, and the
			// container keeps running: live output was lost.
			fake_podman::FakeReply::ChunkedTruncated(vec![frame.clone()])
		} else {
			fake_podman::FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	let out = e.attach_with_index(&file, "web", None).await;
	assert!(
		out.is_err(),
		"an attach that lost its output must not report success"
	);
}

/// The other side of the same rule: a container that stops takes its stream
/// down with it, which is a finished read and not a lost one. The container is
/// running when the attach starts and stopped when the stream ends, which is
/// what the two listings below say.
#[tokio::test]
#[cfg(unix)]
async fn attach_accepts_a_stream_that_ended_with_its_container() {
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::sync::Arc;

	// The first listing resolves the replica to attach to; the second one is
	// the out-of-band re-check that asks whether the container is still up.
	let calls = Arc::new(AtomicUsize::new(0));
	let seen = calls.clone();
	let frame = "\u{1}\0\0\0\0\0\0\u{5}hello".to_string();
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			let state = if seen.fetch_add(1, Ordering::SeqCst) == 0 {
				"running"
			} else {
				"exited"
			};
			fake_podman::FakeReply::Body(
				200,
				format!(
					r#"[{{"Names":["/proj-web-1"],"State":"{state}","Labels":{{"podup.service":"web"}}}}]"#
				),
			)
		} else if method == "GET" && target.contains("/proj-web-1/json") {
			fake_podman::FakeReply::Body(200, r#"{"State":{"Status":"running"}}"#.to_string())
		} else if method == "GET" && target.contains("/proj-web-1/logs") {
			fake_podman::FakeReply::ChunkedTruncated(vec![frame.clone()])
		} else {
			fake_podman::FakeReply::Body(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	e.attach_with_index(&file, "web", None)
		.await
		.expect("a stream that ended with its container is a finished read");
}
