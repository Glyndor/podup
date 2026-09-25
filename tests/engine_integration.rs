//! Integration tests that exercise the engine against a real Podman daemon.
//!
//! I require a reachable Podman for the user namespace assertions. Other groups
//! skip when Podman is unavailable unless `PODUP_REQUIRE_PODMAN=1` is set.
//! I use that setting in the nested-virt lane so an unreachable Podman fails.
//!
//! The test bodies are split across the `engine_integration/` submodules to
//! keep each file under the source line limit. Shared helpers live here at the
//! crate root so the submodules can reach them via `use super::*;`.
use std::fs;

use podup::{parse_files_with_env_files, parse_str, Client, Engine};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Turn podup's own `tracing` output on for the suite, once per test binary.
///
/// The diagnostics added for #1104 and #1097 are `tracing::warn!` calls, and
/// `tracing` is a facade: a `warn!` with no subscriber installed in the process
/// is discarded silently. `main` installs one, but every integration test runs
/// in a process that never calls `main`, so the instrumentation compiled, passed
/// review, merged, and emitted nothing on the lane, which is the one place it
/// was written to answer a question.
///
/// libtest captures output and prints it only for tests that fail, which is
/// exactly the wanted shape: a green run stays quiet, and a red one carries the
/// classification of how the stream actually ended.
fn enable_tracing() {
	static ONCE: std::sync::Once = std::sync::Once::new();
	ONCE.call_once(|| {
		use tracing_subscriber::{fmt, EnvFilter};
		// `warn` covers the diagnostics without the per-request noise of `debug`.
		// RUST_LOG still wins when someone wants more.
		let filter =
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("podup=warn"));
		// `with_test_writer` routes through libtest's capture; a plain writer
		// would bypass it and interleave across parallel tests.
		let _ = fmt().with_env_filter(filter).with_test_writer().try_init();
	});
}

async fn podman() -> Option<Client> {
	enable_tracing();
	let connected =
		match podup::podman::connect_from_env().or_else(|_| podup::podman::connect(None)) {
			Ok(client) => client.ping().await.is_ok().then_some(client),
			Err(_) => None,
		};
	// Skipping is the right default: these tests must not fail on a developer
	// machine without Podman. But a silent skip reports `ok` for a test that
	// executed nothing, and libtest counts it as passed, so an environment
	// where Podman never came up looks identical to a clean run. Somewhere that
	// Podman is guaranteed (the nested-virt lane), set PODUP_REQUIRE_PODMAN and
	// the skip becomes a hard failure instead of a green lie.
	assert!(
		!(connected.is_none() && std::env::var_os("PODUP_REQUIRE_PODMAN").is_some()),
		"PODUP_REQUIRE_PODMAN is set but Podman is unreachable; refusing to report this suite as passing without running it"
	);
	connected
}

/// Unique project name per test run + per test to avoid parallel conflicts.
fn proj(tag: &str) -> String {
	publish_pid_file();
	format!("t{}-{}", std::process::id(), tag)
}

/// One shared mutex for every test that creates a `--userns=auto` allocation:
/// the three container cases in `tests/engine_integration/userns.rs` and the
/// pod case in `tests/engine_integration/userns_pod.rs`. Both modules lock it
/// before they call `up` so two lanes never race on the host's subordinate
/// UID range.
///
/// Measured on Podman 5.7.0 against a 65536-ID subuid range on
/// 2026-09-22: `--userns=auto` hands out 1024-block ranges from the tail of
/// the subuid pool, and the tail is only ~4261 IDs. One `--userns=auto`
/// container takes one of those blocks; `--userns=auto:size=2048` takes two
/// blocks by itself. Without this lock two lanes could easily drive the live
/// count past four concurrent allocations and the next allocation would fail
/// with "not enough unused IDs in user namespace", which reads like a podup
/// defect and is not one. Locking both lanes through the same mutex caps the
/// live count at one allocation at a time.
static USERNS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Path the suite writes its PID to. The CI step reads the same path when
/// setting `PODUP_LEAK_SCAN_PID`; one constant, two readers, no string to
/// keep in step.
const PID_FILE_PATH: &str = "target/podup-leak-scan-pid";

/// Write this run's PID to [`PID_FILE_PATH`] once, on the first call to
/// [`proj`]. Only the PID crosses the process boundary: the leak-scan
/// binary in `tests/integration_leak_scan.rs` rebuilds the three patterns
/// it matches on from the PID alone.
///
/// The write is intentionally eager. The earliest test to create a
/// resource with the per-run prefix will be the first to call `proj`,
/// and the file exists before any resource does. A test that creates
/// resources without going through `proj` cannot carry the per-run
/// prefix, so the ordering is irrelevant for them.
fn publish_pid_file() {
	static ONCE: std::sync::OnceLock<()> = std::sync::OnceLock::new();
	ONCE.get_or_init(|| {
		if let Some(parent) = std::path::Path::new(PID_FILE_PATH).parent() {
			// `target/` exists at this point (cargo creates it before
			// tests run), but `create_dir_all` is a no-op when it does
			// and harmless when it doesn't.
			let _ = std::fs::create_dir_all(parent);
		}
		let _ = std::fs::write(PID_FILE_PATH, std::process::id().to_string());
	});
}

/// Owns a tag podman knows about and removes it on drop.
///
/// The companion to `Engine::down`: the engine tears down containers,
/// networks, volumes and the pod, but a build that tagged a new image leaves
/// it behind, and a `commit` does the same. Bind one at the top of a test and
/// every panic in the body still ends with the image gone, because the guard
/// fires during unwinding and not from the happy path only.
struct TestImage {
	tag: String,
}

impl TestImage {
	fn new(tag: impl Into<String>) -> Self {
		Self { tag: tag.into() }
	}
}

impl Drop for TestImage {
	fn drop(&mut self) {
		let _ = std::process::Command::new("podman")
			.args(["rmi", "-f", &self.tag])
			.output();
	}
}

/// Owns one CLI project: the tempdir the compose lives in, the compose
/// path, and the project name. Drop runs `podup -f <compose> -p <name> down -v`
/// so the network and any volume the test stood up are reaped even when an
/// assertion panics in the middle of the body.
///
/// Several integration tests call the CLI to create a project and never call
/// `down`: a panic in the test body, or just a test that ends with an
/// assertion failure, leaves `<project>_default` on the host. A drop guard is
/// the bound that fires during stack unwinding, where the trailing happy-path
/// `down` calls cannot reach. The shape mirrors `TestImage`, which serves
/// the same role for tagged images left behind by `build`.
struct DownGuard {
	_dir: tempfile::TempDir,
	compose: std::path::PathBuf,
	name: String,
}

impl DownGuard {
	/// Bind a fresh project tagged with `tag`. The prefix `t<PID>-` comes from
	/// the harness so the network any code path creates carries a name the
	/// leak-scan binary can recognise.
	fn new(tag: &str, compose_body: &str) -> Self {
		let dir = tempfile::tempdir().expect("tempdir");
		let compose = dir.path().join("docker-compose.yml");
		std::fs::write(&compose, compose_body).expect("write compose");
		// Through `proj`, not a second spelling of the prefix: `proj` is also
		// what records this run's PID for the leak scan.
		let name = proj(tag);
		Self {
			_dir: dir,
			compose,
			name,
		}
	}

	fn compose_path(&self) -> &str {
		self.compose.to_str().expect("compose path utf8")
	}

	fn name(&self) -> &str {
		&self.name
	}
}

impl Drop for DownGuard {
	fn drop(&mut self) {
		// A failed teardown cannot fail the test from here, but it must not be
		// silent either: the leak scan would report the leftover without
		// saying why. Print the command's own error so the cause is in the
		// test output next to the scan's finding.
		match std::process::Command::new(bin())
			.args(["-f", self.compose_path(), "-p", self.name(), "down", "-v"])
			.output()
		{
			Ok(out) if out.status.success() => {}
			Ok(out) => eprintln!(
				"DownGuard: `down -v` for {} exited {}: {}",
				self.name,
				out.status,
				String::from_utf8_lossy(&out.stderr)
			),
			Err(e) => eprintln!("DownGuard: could not run `down -v` for {}: {e}", self.name),
		}
	}
}

/// Path to the built `podup` binary, for the CLI tests.
fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Run the built `podup` and hand back whatever it did, checking nothing.
///
/// For calls whose outcome the test does not depend on, teardown mostly. When
/// a later assertion depends on this command having worked, use [`run_ok`].
#[allow(dead_code)]
fn run(args: &[&str]) -> std::process::Output {
	std::process::Command::new(bin())
		.args(args)
		.output()
		.unwrap()
}

/// Run the built `podup` and fail with its own words if it did not succeed.
///
/// Setting a test up with [`run`] and then asserting on the effect throws away
/// the evidence of what went wrong. `create_makes_containers_without_starting_them`
/// discarded an `up -d` and reported `left: 0, right: 1`: true, and unable to
/// say whether `up` failed or whether it worked and the container died (#1340).
///
/// The failure is invisible while the environment is healthy and surfaces
/// exactly when something else is already broken, which is when the diagnosis
/// is worth the most.
#[allow(dead_code)]
fn run_ok(args: &[&str]) -> std::process::Output {
	let out = run(args);
	assert!(
		out.status.success(),
		"podup {args:?} exited {}: {}",
		out.status,
		String::from_utf8_lossy(&out.stderr)
	);
	out
}

/// Poll until reading `path` inside `container` yields exactly `expect` once
/// trimmed, or `secs` elapse. Returns whether it matched.
///
/// Reading state back out of the container is how these tests observe an effect
/// that a command's return value cannot show. The usual shape is an entrypoint
/// that appends a line on every start, which makes the file a container-scoped
/// count of how many times the process ran. `/proc/uptime` looks like the
/// obvious alternative and is not one: it is not namespaced, so it reports the
/// host's.
///
/// The comparison is exact rather than a substring, so "started twice" cannot be
/// satisfied by a container that started three times.
/// Poll until a file on the HOST reads exactly `expect` once trimmed, or `secs`
/// elapse. The host side of [`poll_container_file`], for the tests that observe
/// ordering through a bind mount shared by two containers.
async fn poll_host_file(path: std::path::PathBuf, expect: &str, secs: u64) -> bool {
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
	while tokio::time::Instant::now() < deadline {
		if let Ok(out) = std::fs::read_to_string(&path) {
			if out.trim() == expect {
				return true;
			}
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
	false
}

async fn poll_container_file(
	engine: &Engine,
	container: &str,
	path: &str,
	expect: &str,
	secs: u64,
) -> bool {
	let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
	while tokio::time::Instant::now() < deadline {
		if let Ok(out) = engine
			.test_exec_capture(container, vec!["cat".into(), path.into()])
			.await
		{
			if out.trim() == expect {
				return true;
			}
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
	false
}

/// Poll the test's condition until it holds or `timeout` elapses, while
/// watching the spawned watch task. On every tick the helper checks
/// `JoinHandle::is_finished()`; if the task completed, the helper awaits it
/// and panics with its own result so the failure names the real cause (the
/// `Watch` error returned from `Engine::watch`, e.g. an inotify exhaustion)
/// instead of the poll's deadline message, the misleading "never finished"
/// panic the test would otherwise raise when the watcher died silently.
///
/// `make_poll` is called on each tick; return `true` from its future when the
/// condition holds. Return `false` from the helper when the deadline elapsed
/// with the watch task still running, so the existing assertion can decide
/// whether a timeout is a pass or a fail.
///
/// The handle is `&mut` so the helper can `await` it without the test
/// having to take it back, and so a single borrow covers both the
/// `is_finished` check and the `await`. The closure is called fresh on each
/// tick (its future is not stored across iterations), so it can borrow from
/// the test body freely.
async fn poll_with_watch<F, Fut>(
	handle: &mut tokio::task::JoinHandle<podup::Result<()>>,
	timeout: std::time::Duration,
	mut make_poll: F,
) -> bool
where
	F: FnMut() -> Fut,
	Fut: std::future::Future<Output = bool>,
{
	let deadline = tokio::time::Instant::now() + timeout;
	loop {
		if handle.is_finished() {
			// `is_finished` is the only safe way to detect completion
			// without `await`ing first (which would consume the handle);
			// once it returns true, `await` resolves immediately with the
			// task's result.
			match handle.await {
				// `ComposeError::Watch` already prints `watch error:` in its
				// own `Display`; using the inner text here avoids the
				// double prefix and keeps the failure readable.
				Ok(Err(podup::ComposeError::Watch(s))) => {
					panic!("watch() returned early: {s}")
				}
				Ok(Err(e)) => panic!("watch() returned early: {e}"),
				Ok(Ok(())) => panic!("watch() returned early: Ok(())"),
				Err(e) => panic!("watch task panicked: {e}"),
			}
		}
		if make_poll().await {
			return true;
		}
		if tokio::time::Instant::now() >= deadline {
			return false;
		}
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
	}
}

/// Self-test for [`poll_with_watch`]: a spawned task that returns an error
/// immediately must make the helper panic with that error's text, not with
/// the poll's timeout message. The failure mode this guards is the watch
/// task dying silently: the test would otherwise wait out the deadline and
/// panic with "did not copy the file" or similar, blaming the wrong step.
#[tokio::test]
#[should_panic(expected = "watch() returned early: simulated watch failure")]
async fn poll_with_watch_surfaces_a_finished_watch_task_error() {
	let mut handle =
		tokio::spawn(async { Err(podup::ComposeError::Watch("simulated watch failure".into())) });
	// The poll never holds; without the helper's `is_finished` check the
	// timeout would expire and the helper would return `false` instead of
	// panicking. The `#[should_panic]` attribute is what binds the test to
	// the helper's panic contract; sabotaging the helper to skip
	// `is_finished` makes this test fail.
	let _ = poll_with_watch(
		&mut handle,
		std::time::Duration::from_millis(50),
		|| async { false },
	)
	.await;
}

// ---------------------------------------------------------------------------
// Test groups (see engine_integration/*.rs)
// ---------------------------------------------------------------------------

#[path = "engine_integration/autostart_quadlet.rs"]
mod autostart_quadlet;
#[path = "engine_integration/build_images.rs"]
mod build_images;
// Unix only: it reaches the Podman socket by its `/run/user/<uid>` path.
#[cfg(unix)]
#[path = "engine_integration/build_labels.rs"]
mod build_labels;
#[path = "engine_integration/build_resources.rs"]
mod build_resources;
// Unix only: the assertion lists external containers via `podman ps -a
// --external` against a Unix-domain socket, the path the libpod client
// drives.
#[cfg(unix)]
#[path = "engine_integration/build_failure_cleanup.rs"]
mod build_failure_cleanup;
#[path = "engine_integration/build_sparse_context.rs"]
mod build_sparse_context;
#[path = "engine_integration/commands_networking.rs"]
mod commands_networking;
#[path = "engine_integration/cp_flags.rs"]
mod cp_flags;
#[path = "engine_integration/dns_resolution.rs"]
mod dns_resolution;
#[path = "engine_integration/error_surfacing.rs"]
mod error_surfacing;
#[path = "engine_integration/exec_flags.rs"]
mod exec_flags;
#[path = "engine_integration/health_targeting.rs"]
mod health_targeting;
#[path = "engine_integration/implicit_dependencies.rs"]
mod implicit_dependencies;
#[path = "engine_integration/include_extends.rs"]
mod include_extends;
#[path = "engine_integration/label_file_safety.rs"]
mod label_file_safety;
#[path = "engine_integration/lifecycle.rs"]
mod lifecycle;
#[path = "engine_integration/lifecycle_query.rs"]
mod lifecycle_query;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/libpod_origin_form_build_comp.rs"]
mod libpod_origin_form_build_comp;
#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/libpod_origin_form_io_comps.rs"]
mod libpod_origin_form_io_comps;
#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/libpod_origin_form_lifecycle_comps.rs"]
mod libpod_origin_form_lifecycle_comps;
#[path = "engine_integration/niche.rs"]
mod niche;
#[path = "engine_integration/recreate_on_image.rs"]
mod recreate_on_image;
#[path = "engine_integration/resources_health.rs"]
mod resources_health;
#[path = "engine_integration/run_flags.rs"]
mod run_flags;
#[path = "engine_integration/secrets.rs"]
mod secrets;

#[cfg(feature = "test-helpers")]
#[path = "engine_integration/watch.rs"]
mod watch_tests;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/watch_sparse.rs"]
mod watch_sparse;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/watch_delete.rs"]
mod watch_delete;

#[cfg(all(unix, feature = "test-helpers"))]
#[path = "engine_integration/watch_ignore_relative.rs"]
mod watch_ignore_relative;

#[path = "engine_integration/x_podman_autoupdate.rs"]
mod x_podman_autoupdate;

#[path = "engine_integration/cli_commands.rs"]
mod cli_commands;
#[path = "engine_integration/cli_flags.rs"]
mod cli_flags;
#[path = "engine_integration/cli_lifecycle.rs"]
mod cli_lifecycle;
#[path = "engine_integration/cli_output.rs"]
mod cli_output;
#[path = "engine_integration/create_ls.rs"]
mod create_ls;
#[path = "engine_integration/lifecycle_output.rs"]
mod lifecycle_output;
#[path = "engine_integration/logs_reader_closes.rs"]
mod logs_reader_closes;
#[path = "engine_integration/multi_file.rs"]
mod multi_file;
#[path = "engine_integration/network_ownership.rs"]
mod network_ownership;
/// A free loopback port, chosen by binding zero and releasing it.
///
/// Shared because three tests hard-coded `18081` and a fourth `18080`, so any
/// two of them running at once fought over the same bind and the loser failed
/// with `pasta failed ... Address already in use`. That is not flakiness: at
/// eight test threads it is close to certain.
///
/// There is a window between releasing the port and the container binding it.
/// It is small, and far smaller than the certainty of a shared constant.
#[allow(dead_code)]
fn free_port() -> u16 {
	std::net::TcpListener::bind("127.0.0.1:0")
		.expect("no loopback port")
		.local_addr()
		.unwrap()
		.port()
}

#[path = "engine_integration/push_registry.rs"]
mod push_registry;
#[path = "engine_integration/scale.rs"]
mod scale;
#[path = "engine_integration/start_restores_config.rs"]
mod start_restores_config;
#[path = "engine_integration/stats_flags.rs"]
mod stats_flags;

#[path = "engine_integration/x_podman_pod.rs"]
mod x_podman_pod;

#[path = "engine_integration/userns.rs"]
mod userns;

#[path = "engine_integration/userns_pod.rs"]
mod userns_pod;

// ---------------------------------------------------------------------------
// Shared helpers for the libpod origin-form compensation tests
// (engine_integration/libpod_origin_form_*.rs). One helper file per concern
// would be cleaner, but these helpers are small enough that colocating them
// with the rest of the crate-root helpers is the simpler split, and the tests
// that use them (`super::*`) reach the crate root the same way every other
// test group already does.
//
// The whole block is gated `cfg(all(unix, feature = "test-helpers"))` to
// match the three `libpod_origin_form_*` modules it serves, which carry the
// same gate at the `mod` declarations above. `podman_socket_url` reaches
// `libc::getuid()` to build the `/run/user/<uid>/podman/podman.sock` path;
// `libc` is a `[target.'cfg(unix)'.dependencies]` line in `Cargo.toml`, so on
// Windows the crate is not in scope and the test target fails to compile
// with `error[E0433]: cannot find module or crate libc`. Every caller of
// every helper here is inside one of the three libpod modules, so the gate
// is exact and no other test group loses anything.
// ---------------------------------------------------------------------------

/// Locate the Podman socket the engine talks to. The CLI's own storage root
/// is often different from the socket's (a fresh CLI invocation on Linux
/// resolves to a tmpfs path the socket does not share), so plain
/// `podman ps` queries the wrong store on most setups. The CLI's `--url`
/// flag forwards the request to the socket instead, which is what the
/// live tests inspect.
///
/// Returns `None` when no candidate socket exists; the live tests skip on
/// that path.
#[cfg(all(unix, feature = "test-helpers"))]
pub(crate) fn podman_socket_url() -> Option<String> {
	for path in [
		format!("/run/user/{}/podman/podman.sock", unsafe { libc::getuid() }),
		"/run/podman/podman.sock".to_string(),
	] {
		if std::path::Path::new(&path).exists() {
			return Some(format!("unix://{path}"));
		}
	}
	None
}

/// Run `podman --url <socket> <args...>` and return the trimmed stdout.
/// Panics with stderr on a non-zero exit so a failing assertion carries the
/// actual Podman response.
#[cfg(all(unix, feature = "test-helpers"))]
pub(crate) fn podman_cmd(socket: &str, args: &[&str]) -> String {
	let out = std::process::Command::new("podman")
		.args(["--url", socket])
		.args(args)
		.output()
		.unwrap_or_else(|e| panic!("podman {args:?}: {e}"));
	if !out.status.success() {
		panic!(
			"`podman --url {socket} {args:?}` exited {}: {}",
			out.status,
			String::from_utf8_lossy(&out.stderr)
		);
	}
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Build a project on the live socket and start it. Returns
/// `(tempdir, project_name, container_name)` so the compose file outlives
/// the `up` and the test can reach the project's running container. The
/// composition drives the `podup` binary through `CARGO_BIN_EXE_podup`, the
/// same binary `cargo test --test engine_integration` resolves at build
/// time.
#[cfg(all(unix, feature = "test-helpers"))]
pub(crate) fn up_service(
	socket: &str,
	tag: &str,
	body: &str,
) -> (tempfile::TempDir, String, String) {
	let dir = tempfile::tempdir().expect("tempdir");
	let compose = dir.path().join("compose.yaml");
	std::fs::write(&compose, body).expect("write compose");
	let name = format!("t{}-{}", std::process::id(), tag);
	let bin = bin();
	let out = std::process::Command::new(bin)
		.args(["-f"])
		.arg(&compose)
		.args(["-p", &name, "up", "-d", "--no-build"])
		.env("PODMAN_SOCKET", socket)
		.output()
		.expect("run podup up");
	assert!(
		out.status.success(),
		"`podup up` failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let container = podman_cmd(
		socket,
		&[
			"ps",
			"-a",
			"--format",
			"{{.Names}}",
			"--filter",
			&format!("label=podup.project={name}"),
		],
	);
	let container = container
		.lines()
		.next()
		.unwrap_or_default()
		.trim_start_matches('/')
		.to_string();
	assert!(
		!container.is_empty(),
		"no project container was created for {name}"
	);
	(dir, name, container)
}

/// Tear the project down. Best-effort: the `Drop` on `DownGuard` would do
/// the same, but a single explicit teardown keeps the assertion surface
/// (and the leftover list) clean.
#[cfg(all(unix, feature = "test-helpers"))]
pub(crate) fn down(socket: &str, dir: &tempfile::TempDir, name: &str) {
	let compose = dir.path().join("compose.yaml");
	let _ = std::process::Command::new(bin())
		.args(["-f"])
		.arg(&compose)
		.args(["-p", name, "down", "-v"])
		.env("PODMAN_SOCKET", socket)
		.output();
}

/// Time the wall-clock between two instants in milliseconds.
#[cfg(all(unix, feature = "test-helpers"))]
pub(crate) fn elapsed_ms(start: std::time::Instant) -> u128 {
	start.elapsed().as_millis()
}
