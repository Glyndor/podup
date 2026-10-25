//! What every test group in this target shares: the Podman connection, the
//! per-run project name, the two mutexes that serialise lanes against each
//! other, the drop guards, and the built-binary driver.
use super::*;

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

pub(crate) async fn podman() -> Option<Client> {
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
pub(crate) fn proj(tag: &str) -> String {
	publish_pid_file();
	format!("t{}-{}", std::process::id(), tag)
}

/// One shared mutex for every test that creates a `--userns=auto` allocation:
/// any test that brings up a container or pod with `userns_mode: auto` takes
/// this lock before it calls `up` and holds it through teardown, so two lanes
/// never race on the host's subordinate UID range.
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
pub(crate) static USERNS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One shared mutex for every test that builds an image. The test takes this
/// lock at the start of its body and holds it through its image cleanup
/// (`podman rmi -f` of the tag it just produced), so no two build tests
/// overlap. Tests that do not build an image do not take it, and builders in
/// other test files do not take it.
///
/// Measured on the real store (Podman 5.7.0, eight test threads,
/// 2026-09-28):
///
/// - `build_images` only, five runs: 13/13 each.
/// - Lock disabled: two of three runs failed four tests with
///   `Build("checking if cached image exists from a previous build:
///   getting top layer info: layer not known ...")` (the #1960 defect).
/// - Full `engine_integration` at eight threads, two runs: 242 passed and
///   one failed both times (`pod_userns_mode::a_pod_takes_the_services_user_namespace`,
///   which competes for `--userns=auto` slots; that test takes `USERNS`).
///
/// The cause has not been isolated with the lock removed, but the most
/// likely one is a build's cache lookup racing another test's
/// `podman rmi -f` of an image it shares layers with: the lookup returns
/// `layer not known` and the build aborts. Locking the build tests through
/// one mutex keeps a `rmi -f` from running while another build is doing that
/// lookup.
pub(crate) static BUILD_IMAGES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
pub(crate) struct TestImage {
	tag: String,
}

impl TestImage {
	pub(crate) fn new(tag: impl Into<String>) -> Self {
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
pub(crate) struct DownGuard {
	_dir: tempfile::TempDir,
	compose: std::path::PathBuf,
	name: String,
}

impl DownGuard {
	/// Bind a fresh project tagged with `tag`. The prefix `t<PID>-` comes from
	/// the harness so the network any code path creates carries a name the
	/// leak-scan binary can recognise.
	pub(crate) fn new(tag: &str, compose_body: &str) -> Self {
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

	pub(crate) fn compose_path(&self) -> &str {
		self.compose.to_str().expect("compose path utf8")
	}

	pub(crate) fn name(&self) -> &str {
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
pub(crate) fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Run the built `podup` and hand back whatever it did, checking nothing.
///
/// For calls whose outcome the test does not depend on, teardown mostly. When
/// a later assertion depends on this command having worked, use [`run_ok`].
#[allow(dead_code)]
pub(crate) fn run(args: &[&str]) -> std::process::Output {
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
pub(crate) fn run_ok(args: &[&str]) -> std::process::Output {
	let out = run(args);
	assert!(
		out.status.success(),
		"podup {args:?} exited {}: {}",
		out.status,
		String::from_utf8_lossy(&out.stderr)
	);
	out
}

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
pub(crate) fn free_port() -> u16 {
	std::net::TcpListener::bind("127.0.0.1:0")
		.expect("no loopback port")
		.local_addr()
		.unwrap()
		.port()
}

/// The image ID a tag currently resolves to, via the podman CLI, the same
/// out-of-band check the sibling build tests use to observe state podup
/// itself reports on. Empty when the tag is absent.
pub(crate) fn image_id(tag: &str) -> String {
	let out = std::process::Command::new("podman")
		.args(["inspect", tag, "--format", "{{.Id}}"])
		.output()
		.expect("run podman inspect");
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// How many containers `ps -a -q` reports for `proj`, counted non-empty lines.
pub(crate) fn ps_all_count(compose: &str, proj: &str) -> usize {
	String::from_utf8_lossy(&run(&["-f", compose, "-p", proj, "ps", "-a", "-q"]).stdout)
		.lines()
		.filter(|l| !l.trim().is_empty())
		.count()
}
