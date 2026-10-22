// ---------------------------------------------------------------------------
// Shared helpers for the libpod origin-form compensation tests
// (engine_integration/libpod_origin_form_*.rs). Re-exported from the crate
// root, so the modules that use them keep reaching them through `super::*`
// like every other test group.
//
// The whole block is gated `cfg(all(unix, feature = "test-helpers"))` to
// match the four `libpod_origin_form_*` modules it serves, which carry the
// same gate at the `mod` declarations in the crate root.
// `podman_socket_url` reaches `libc::getuid()` to build the
// `/run/user/<uid>/podman/podman.sock` path;
// `libc` is a `[target.'cfg(unix)'.dependencies]` line in `Cargo.toml`, so on
// Windows the crate is not in scope and the test target fails to compile
// with `error[E0433]: cannot find module or crate libc`. Every caller of
// every helper here is inside one of the four libpod modules, so the gate
// is exact and no other test group loses anything.
// ---------------------------------------------------------------------------

use super::*;

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
