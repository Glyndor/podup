//! The exit code the `audit` subcommand returns: what `--strict` changes and
//! when the gate is allowed to stay green.
//!
//! Split out from `tests/audit/audit_exit_codes.rs` so the parent file stays under
//! the repository line limit; the helpers (`run`, `write_compose`) are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// exit codes
// ---------------------------------------------------------------------------

#[test]
fn audit_exits_zero_with_findings_by_default() {
	// A trivially unhardened service: every check that fires on a bare
	// `image: nginx` will surface. Without `--strict` the exit code must
	// stay 0, the same way `ps` exits 0 on a stopped project, so a CI
	// pipeline that has not opted in sees the report without breaking.
	let path = write_compose("services:\n  web:\n    image: nginx\n    privileged: true\n");
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit"]);
	assert!(
		out.status.success(),
		"`audit` must default to exit 0; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
	// The table is the whole point of the default run; a sweep that emptied
	// the renderer left this test green, so the output is read too.
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		stdout.contains("SERVICE") && stdout.contains("FINDINGS"),
		"no table header:\n{stdout}"
	);
	assert!(
		stdout.contains("writable_root"),
		"no finding named in the table:\n{stdout}"
	);
	assert!(
		stdout.contains(": writable_root: "),
		"no reason line under the table:\n{stdout}"
	);
}

#[test]
fn audit_strict_exits_one_with_findings() {
	// Same compose file, `--strict` enabled: the same findings should now
	// flip the exit to 1. CI scripts pipe `--strict` into a job's success
	// gate, so this is the property they care about.
	let path = write_compose("services:\n  web:\n    image: nginx\n    privileged: true\n");
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--strict"]);
	assert_eq!(
		out.status.code(),
		Some(1),
		"`--strict` with findings must exit 1; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
}

#[test]
fn audit_strict_exits_zero_when_clean() {
	// A fully hardened service must exit 0 even with `--strict`, otherwise
	// the CI-gate promise is "fail forever" rather than "fail when there
	// is something to fix". The unit suite already pins per-check
	// pass/warn behaviour; this is the CLI-level sum of that.
	let path = write_compose(
		r#"services:
  web:
    image: nginx:1.27@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e
    read_only: true
    cap_drop: [ALL]
    security_opt: [no-new-privileges:true]
    pids_limit: 200
    mem_limit: 512m
    memswap_limit: 512m
    init: true
    restart: unless-stopped
    cpus: "1"
    healthcheck:
      test: ["CMD", "true"]
      x-podman-on-failure: restart
    userns_mode: auto
    environment:
      - LOG_LEVEL=info
"#,
	);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--strict"]);
	assert!(
		out.status.success(),
		"`--strict` with no findings must exit 0; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
}
