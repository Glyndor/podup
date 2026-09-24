//! Wire-shape tests for the query-string compensations the libpod endpoint
//! shape requires.
//!
//! The Docker compat handler and the libpod handler share the URL path but
//! differ on which query keys they read. podup used to talk to libpod
//! over the absolute-form request line, which routed every call to the
//! Docker compat handlers, where the docker-side query keys were the
//! ones in scope. Switching the request line to origin form moves
//! every call to the libpod handlers, which read a different subset:
//! `timeout=` (not `t=`), `volumes=` (not `v=`), `copyUIDGID=false`
//! (the default flips), `ps_args=-ef` (the default flips), and
//! `layers=true` on the build endpoint (the default flips). Each
//! compensation here asserts that the call carries the libpod key and
//! not the docker key, on the wire shape captured by the fake podman
//! the build/lifecycle tests already use.

#![cfg(unix)]

use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::Engine;
use crate::libpod::API_PREFIX;
use std::sync::{Arc, Mutex};

fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

/// `POST /containers/{}/stop` must carry `timeout=` and not `t=`. The
/// libpod handler reads `timeout=`; the Docker compat handler reads
/// `t=`, so an unchanged `t=` query key leaves the libpod handler
/// ignoring the grace period entirely (defaulting to the container's
/// own stop timeout, or to the daemon default when there is none).
/// This is the unit test for the compensation in `commands.rs::stop_container`.
#[tokio::test]
async fn stop_sends_timeout_query_param() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "POST" && target.contains("/stop?") {
			FakeReply::Body(204, String::new())
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#
					.into(),
			)
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = engine_with(fake.client(), "proj");

	// A service whose `stop_grace_period` is 7 seconds. The helper pins the
	// value the libpod side must see.
	let file =
		crate::parse_str("services:\n  web:\n    image: x\n    stop_grace_period: 7s\n").unwrap();
	engine
		.stop(&file, &["web".into()])
		.await
		.expect("a stop the fake accepts succeeds");

	let stop_req = {
		let requests = requests.lock().unwrap();
		requests
			.iter()
			.find(|r| r.starts_with("POST ") && r.contains("/stop?"))
			.expect("a /stop request was issued")
			.clone()
	};
	let query = stop_req
		.split_once('?')
		.expect("the stop target carries a query string");
	assert!(
		query.1.contains("timeout=7"),
		"the libpod stop endpoint must read `timeout=`, not `t=`: {stop_req:?}"
	);
	assert!(
		!query.1.split('&').any(|pair| pair.starts_with("t=")),
		"the docker `t=` must not appear alongside `timeout=` on libpod: {stop_req:?}"
	);
}

/// `POST /containers/{}/restart` must carry `timeout=` and not `t=`. The
/// libpod handler ignores `t=` and defaults `timeout=` to 0 when absent,
/// which would make every restart an immediate SIGKILL (a single-replica
/// `restart` on libpod would never give the container a chance to
/// drain). This is the unit test for `parallel.rs::restart_one_service`
/// and the watch restart at `watch/mod.rs::watch_restart`.
#[tokio::test]
async fn restart_sends_timeout_query_param() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "POST" && target.contains("/restart?") {
			FakeReply::Body(204, String::new())
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#
					.into(),
			)
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = engine_with(fake.client(), "proj");

	let file =
		crate::parse_str("services:\n  web:\n    image: x\n    stop_grace_period: 5s\n").unwrap();
	engine
		.restart(&file, Some("web"))
		.await
		.expect("a restart the fake accepts succeeds");

	let restart_req = {
		let requests = requests.lock().unwrap();
		requests
			.iter()
			.find(|r| r.starts_with("POST ") && r.contains("/restart?"))
			.expect("a /restart request was issued")
			.clone()
	};
	let query = restart_req
		.split_once('?')
		.expect("the restart target carries a query string");
	assert!(
		query.1.contains("timeout="),
		"the libpod restart endpoint must read `timeout=`: {restart_req:?}"
	);
	assert!(
		!query.1.split('&').any(|pair| pair.starts_with("t=")),
		"the docker `t=` must not appear alongside `timeout=` on libpod: {restart_req:?}"
	);
}

/// `DELETE /containers/{}` must carry `volumes=` (not `v=`) when the
/// caller asked for anonymous-volume removal. The libpod handler
/// reads `volumes=`; the Docker compat handler reads `v=`, which the
/// libpod handler ignores, so a `down -v` against libpod without
/// `volumes=` reclaims nothing. This is the unit test for the
/// `container_rm_path` helper.
#[tokio::test]
async fn down_v_sends_volumes_query_param() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "DELETE" && target.contains("/containers/") {
			FakeReply::Body(204, String::new())
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#
					.into(),
			)
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = engine_with(fake.client(), "proj");

	let file = crate::parse_str("services:\n  web:\n    image: x\n").unwrap();
	engine
		.down_with_options(&file, true)
		.await
		.expect("a down the fake accepts succeeds");

	let del_req = {
		let requests = requests.lock().unwrap();
		requests
			.iter()
			.find(|r| r.starts_with("DELETE ") && r.contains("/containers/proj-web-1?"))
			.expect("a DELETE for the container was issued")
			.clone()
	};
	let query = del_req
		.split_once('?')
		.expect("the delete target carries a query string");
	assert!(
		query.1.contains("volumes=true"),
		"libpod delete must read `volumes=true`, not `v=true`: {del_req:?}"
	);
	assert!(
		!query.1.split('&').any(|pair| pair == "v=true"),
		"the docker `v=true` must not appear alongside `volumes=true` on libpod: {del_req:?}"
	);
	assert!(
		query.1.contains("force=true"),
		"`down` tears down a running container, so `force=true` must still be present: {del_req:?}"
	);
}

/// `GET /containers/{}/top` must carry `ps_args=-ef`. The libpod handler
/// defaults `ps_args` to its own descriptors (a different column
/// set), so podup's user-facing `top` output would drift the moment
/// a service moved off a default-configured image.
#[tokio::test]
async fn top_sends_ps_args_query_param() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "GET" && target.contains("/top?") {
			FakeReply::Body(
				200,
				r#"{"Titles":["UID","PID","PPID","C","STIME","TTY","TIME","CMD"],"Processes":[["root","1","0","0","15:00","?","00:00:00","sleep 3600"]]}"#
					.into(),
			)
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#
					.into(),
			)
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = engine_with(fake.client(), "proj");

	let file = crate::parse_str("services:\n  web:\n    image: x\n").unwrap();
	engine
		.top_with_options(&file, &[], false)
		.await
		.expect("a top the fake accepts succeeds");

	let top_req = {
		let requests = requests.lock().unwrap();
		requests
			.iter()
			.find(|r| r.starts_with("GET ") && r.contains("/top?"))
			.expect("a /top request was issued")
			.clone()
	};
	let query = top_req
		.split_once('?')
		.expect("the top target carries a query string");
	assert!(
		query.1.contains("ps_args=-ef"),
		"the libpod top endpoint must read `ps_args=-ef`: {top_req:?}"
	);
}

/// `PUT /containers/{}/archive` must carry `copyUIDGID=false`. The
/// libpod handler defaults `copyUIDGID` to true, which makes a host
/// file copied into a container take the container's runtime UID/GID
/// (i.e. `0:0`). The Docker compat handler defaulted it to false,
/// which preserved the host UID/GID on the destination file (the
/// Step 0 measurement: `1000:1000`). The compensation pins the
/// docker-compat default.
///
/// The wire-level test for the compensation lives next to the rest of
/// the `cp` upload tests in `engine::copy::upload_tests::upload_carries_copy_uid_gid_false`,
/// which drives the same fake podman through the streaming packer
/// `cp_to_container` uses (the packer is private to the `copy`
/// module). What is pinned here is the URL helper the production path
/// is built on, so a regression that dropped the parameter is caught
/// even when the streaming packer cannot be reached.
#[test]
fn archive_put_path_includes_copy_uid_gid_false() {
	let path = crate::engine::copy::upload::archive_put_path("proj-web-1", "/tmp");
	assert!(
		path.contains("copyUIDGID=false"),
		"libpod archive PUT must carry `copyUIDGID=false`: {path}"
	);
	assert!(
		!path.split('&').any(|pair| pair == "copyUIDGID=true"),
		"the libpod default of `copyUIDGID=true` must not be sent as-is: {path}"
	);
}

/// `POST /containers/{}/kill?signal=SIGKILL` must be followed by a
/// `POST /containers/{}/wait?condition=stopped` on libpod. The Docker
/// compat `/kill` handler blocks on the container when the signal is
/// SIGKILL/9/KILL/0; the libpod handler replies immediately. Without
/// the follow-up wait, a caller that relied on the compat handler's
/// semantics would observe a still-running container when `kill`
/// returned. SIGTERM and the other graceful signals do not block
/// either way and the wait is skipped.
#[tokio::test]
async fn kill_with_sigkill_sends_follow_up_wait() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "POST" && target.contains("/kill?") {
			FakeReply::Body(204, String::new())
		} else if method == "POST" && target.contains("/wait?") {
			// The wait endpoint returns an HTTP 200 with the exit code as
			// its body; a plain `Body(200, "0")` is the libpod shape.
			FakeReply::Body(200, "0".to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#
					.into(),
			)
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = engine_with(fake.client(), "proj");

	let file = crate::parse_str("services:\n  web:\n    image: x\n").unwrap();
	engine
		.kill(&file, &[], "SIGKILL")
		.await
		.expect("a kill the fake accepts succeeds");

	let requests = requests.lock().unwrap();
	let kill_req = requests
		.iter()
		.find(|r| r.starts_with("POST ") && r.contains("/kill?"))
		.expect("a /kill request was issued");
	assert!(
		kill_req.contains("signal=SIGKILL"),
		"the kill must carry the requested signal: {kill_req:?}"
	);
	let wait_req = requests
		.iter()
		.find(|r| r.starts_with("POST ") && r.contains("/wait?"))
		.expect("a /wait?condition=stopped follow-up must follow the kill");
	assert!(
		wait_req.contains("condition=stopped"),
		"the follow-up wait must pin the container's stopped state: {wait_req:?}"
	);
}

/// The opposite case: SIGTERM is graceful and the libpod handler
/// replies promptly anyway, so a `kill -s SIGTERM <id>` must NOT pin
/// the caller behind a per-container `/wait?condition=stopped`. The
/// compensation's contract is that the wait fires for SIGKILL/9/KILL/0
/// only; adding it for SIGTERM is what would turn a 50-replica `kill`
/// into 50 sequential waits.
#[tokio::test]
async fn kill_with_sigterm_skips_the_follow_up_wait() {
	let requests: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
	let req_clone = requests.clone();
	let fake = fake_podman::start_replying(move |method, target| {
		req_clone.lock().unwrap().push(format!("{method} {target}"));
		if method == "POST" && target.contains("/kill?") {
			FakeReply::Body(204, String::new())
		} else if method == "GET" && target.contains("/containers/json") {
			FakeReply::Body(
				200,
				r#"[{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}}]"#
					.into(),
			)
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});
	let engine = engine_with(fake.client(), "proj");

	let file = crate::parse_str("services:\n  web:\n    image: x\n").unwrap();
	engine
		.kill(&file, &[], "SIGTERM")
		.await
		.expect("a kill the fake accepts succeeds");

	let requests = requests.lock().unwrap();
	assert!(
		requests
			.iter()
			.any(|r| r.starts_with("POST ") && r.contains("/kill?signal=SIGTERM")),
		"a /kill?signal=SIGTERM must be issued: {requests:?}"
	);
	assert!(
		!requests
			.iter()
			.any(|r| r.starts_with("POST ") && r.contains("/wait?")),
		"SIGTERM must not trigger a follow-up /wait?condition=stopped: {requests:?}"
	);
}

/// Spot-check the constants and the request path used by the build
/// endpoint. The build endpoint itself is covered by
/// `engine::build::query_tests::build_query_carries_layers_true`;
/// what is pinned here is that the URL the build request is built
/// against still points at the libpod `/build` path (a regression
/// here would mean the compensation went to the wrong place).
#[tokio::test]
async fn build_request_path_targets_libpod_build() {
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "POST" && target.contains("/build?") {
			FakeReply::ChunkedEnd(vec![
				"{\"stream\":\"--> sha256:1111111111111111111111111111111111111111111111111111111111111111\\n\"}\n".to_string(),
				"{\"stream\":\"Successfully tagged proj/img:1\\n\"}\n".to_string(),
			])
		} else if method == "POST" && target.contains("/images/") && target.contains("/tag") {
			FakeReply::Body(200, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});

	let dir = tempfile::tempdir().unwrap();
	let ctx = dir.path().to_path_buf();
	std::fs::write(ctx.join("Dockerfile"), b"FROM alpine:latest\nRUN echo hi\n").unwrap();
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), ctx.clone());
	let file = crate::parse_str(
		"services:\n  app:\n    image: proj/img:1\n    build:\n      context: .\n",
	)
	.unwrap();
	engine
		.build_all_with_options(&file, &[], &crate::engine::BuildOptions::default())
		.await
		.expect("a build the fake accepts succeeds");

	// The full build path lands at `${API_PREFIX}/build?...`; a regression
	// that moved the target outside `/libpod/` would let the docker
	// compat handler answer again and the `layers=true` compensation be
	// for nothing.
	let requests = fake.requests.lock().unwrap();
	assert!(
		requests
			.iter()
			.any(|r| { r.starts_with("POST ") && r.contains(&format!("{API_PREFIX}/build?")) }),
		"a POST to `{API_PREFIX}/build?...` must be issued: {requests:?}"
	);
}
