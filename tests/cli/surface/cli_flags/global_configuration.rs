//! The globals that name a place or a socket are checked: a project
//! directory that does not exist, a compose file that does not exist, and a
//! `DOCKER_HOST` pointing at something remote are all errors rather than
//! silent no-ops.
//!
//! Split out from `tests/cli/surface/cli_flags.rs`; the helper (`bin`, `run_offline`) is
//! inherited via `use super::*`.

use super::*;

#[test]
fn missing_project_directory_is_rejected() {
	let out = Command::new(bin())
		.args(["--project-directory", "/no/such/podup/dir", "config"])
		.output()
		.expect("run with bad project-directory");
	assert!(
		!out.status.success(),
		"missing project directory must error"
	);
	assert!(
		String::from_utf8_lossy(&out.stderr).contains("project-directory"),
		"error should name --project-directory:\n{}",
		String::from_utf8_lossy(&out.stderr)
	);
}

#[test]
fn docker_host_remote_scheme_is_rejected() {
	let out = Command::new(bin())
		.arg("ls")
		.env_remove("PODMAN_SOCKET")
		.env("DOCKER_HOST", "tcp://127.0.0.1:2375")
		.output()
		.expect("run ls with DOCKER_HOST");
	assert!(!out.status.success(), "remote DOCKER_HOST must error");
	assert!(
		String::from_utf8_lossy(&out.stderr).contains("remote Podman"),
		"DOCKER_HOST=tcp:// should be rejected as remote:\n{}",
		String::from_utf8_lossy(&out.stderr)
	);
}

/// `ps` and `events` run without a compose file so a directory with none
/// still answers by label, but a file named with `-f` that is not there is
/// a typo, not an absence: it fails the way every other command fails
/// (#1687). Measured before the fix: `ps` printed `no containers` and exited 0.
#[test]
fn a_named_compose_file_that_is_missing_fails_ps_and_events() {
	for cmd in ["ps", "events"] {
		let out = run_offline(&["-f", "/nonexistent-podup-compose.yml", cmd]);
		let stderr = String::from_utf8_lossy(&out.stderr);
		assert!(
			!out.status.success(),
			"`-f <missing> {cmd}` must fail; stdout: {}",
			String::from_utf8_lossy(&out.stdout)
		);
		assert!(
			stderr.contains("compose file not found"),
			"`-f <missing> {cmd}` names the missing file; got: {stderr}"
		);
	}
}
