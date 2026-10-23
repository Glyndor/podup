#![cfg(feature = "update")]
//! A compose-only global flag passed to `update` is a usage error, rejected
//! before any network access.
//!
//! Split out from `tests/cli_diagnostics.rs`; the helper `bin` is inherited
//! via `use super::*`.

use super::*;

/// `update` only rewrites the podup binary, so the compose-only global value
/// flags (--socket/--profile/--env-file/--project-directory) cannot affect it.
/// Passing one on the command line is rejected as a usage error rather than
/// silently accepted as a no-op, and the rejection happens before any network
/// access, so the test never reaches GitHub.
///
/// Gated on the `update` feature: package-manager builds (Debian/apt) compile
/// without it, so the `update` subcommand does not exist there.
#[cfg(feature = "update")]
#[test]
fn update_rejects_compose_only_global_flags() {
	for flag in [
		"--socket=unix:///tmp/nope.sock",
		"--profile=dev",
		"--env-file=/tmp/nope.env",
		"--project-directory=/tmp",
	] {
		let output = Command::new(bin())
			.args([flag, "update", "--check"])
			.env_remove("PODMAN_SOCKET")
			.env_remove("COMPOSE_PROFILES")
			.output()
			.expect("run podup update with a compose-only flag");
		assert!(
			!output.status.success(),
			"`{flag}` must be rejected for update, got success"
		);
		let stderr = String::from_utf8_lossy(&output.stderr);
		assert!(
			stderr.contains("no effect on `update`"),
			"rejection should explain the misuse for {flag}; got stderr:\n{stderr}"
		);
	}
}
