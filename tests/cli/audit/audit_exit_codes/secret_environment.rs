//! `secret_in_environment`: which keys fire the finding and which stay
//! silent.
//!
//! Split out from `tests/cli/audit/audit_exit_codes.rs` so the parent file stays under
//! the repository line limit; the helpers (`run`, `write_compose`) are
//! inherited via `use super::*`.

use super::*;

#[test]
fn audit_secret_in_environment_flags_literal_secret() {
	// The original behaviour we keep: a literal value under a
	// secret-bearing key still fires, and the literal value is never
	// echoed back.
	let body = "services:\n  web:\n    image: alpine:3.20\n    environment:\n      - DB_PASSWORD=hunter2\n";
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--strict"]);
	assert_eq!(
		out.status.code(),
		Some(1),
		"literal DB_PASSWORD=hunter2 must fail --strict; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(stdout.contains("secret_in_environment"), "stdout: {stdout}");
	assert!(
		!stdout.contains("hunter2"),
		"the literal must not be echoed: {stdout}"
	);
}

#[test]
fn audit_secret_in_environment_does_not_flag_passthrough_or_empty() {
	// Two unrelated values that stay silent. `PASSTHROUGH` has no
	// secret-bearing segment (its segments are `[PASSTHROUGH]`), so the
	// check has nothing to match on; an empty literal under a non-secret
	// key is unrelated to the check entirely.
	let body = "services:\n  web:\n    image: alpine:3.20\n    environment:\n      - PASSTHROUGH=true\n      - LOG_LEVEL=\n";
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--format", "json"]);
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		!stdout.contains("secret_in_environment"),
		"PASSTHROUGH=true and an empty non-secret key must not fire: {stdout}"
	);
}
