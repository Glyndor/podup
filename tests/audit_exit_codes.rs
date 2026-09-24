//! End-to-end checks for the `podup audit` subcommand's exit codes and JSON
//! shape. Drives the compiled `podup` binary against a tiny compose file on
//! disk so the parsing, profile honouring, and emit path are exercised the
//! same way an operator would; the unit suite already covers the checks
//! themselves in isolation.
//!
//! Names state the contract: `--strict` flips the exit code from 0
//! to 1 when any finding is present, and `--format json` is the
//! machine-readable surface CI consumes.

use std::fs;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Render `body` to a unique tempfile under the system tempdir and return
/// its path. The audit command is invoked with `-f <path>` so the compose
/// file lives exactly where the operator would point `-f`. Every call gets
/// a freshly-numbered subdirectory; the counter is atomic so two threads
/// running the same test in parallel never share a directory.
fn write_compose(body: &str) -> std::path::PathBuf {
	static COUNTER: AtomicU64 = AtomicU64::new(0);
	let n = COUNTER.fetch_add(1, Ordering::Relaxed);
	let dir = std::env::temp_dir().join(format!("podup-audit-{}-{n}", std::process::id()));
	let _ = fs::remove_dir_all(&dir);
	fs::create_dir_all(&dir).unwrap();
	let path = dir.join("compose.yaml");
	fs::write(&path, body).unwrap();
	path
}

fn run(args: &[&str]) -> Output {
	let mut cmd = Command::new(bin());
	cmd.args(args);
	// The integration tests share the developer's shell environment, so any
	// pre-set `PODUP_*` / `COMPOSE_*` would leak into the spawned binary and
	// break parsing or change the resolved project. Strip the env vars podup
	// treats as global configuration; `PATH` and the locale stay so the
	// process can locate shared libraries and render messages.
	for key in [
		"PODUP_LIBPOD_POOL",
		// `PODUP_LIBCOD_POOL` is the legacy typo'd spelling; the runtime
		// still reads it as a fallback so a developer's exported value
		// would otherwise leak into the spawned binary and silently
		// override its default pool size.
		"PODUP_LIBCOD_POOL",
		"PODMAN_SOCKET",
		"DOCKER_HOST",
		"COMPOSE_PROJECT_NAME",
		"COMPOSE_PROFILES",
		"COMPOSE_FILE",
		"NO_COLOR",
	] {
		cmd.env_remove(key);
	}
	cmd.output().expect("run podup audit")
}

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

// ---------------------------------------------------------------------------
// JSON shape: every finding listed, stable schema, no escapes.
// ---------------------------------------------------------------------------

#[test]
fn audit_json_lists_every_finding() {
	// Three services, each deliberately failing a different check. The
	// JSON output must carry every one of them: a regression where a check
	// silently no-ops in JSON (while still appearing in the table) would
	// pass the unit tests and break CI consumers in production.
	let path = write_compose(
		r#"services:
  pr:
    image: nginx
  caps:
    image: nginx:1.27
    cap_add: [SYS_ADMIN]
  secret:
    image: nginx:1.27
    environment:
      - DB_PASSWORD=hunter2
"#,
	);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--format", "json"]);
	assert!(out.status.success(), "audit must succeed");
	let stdout = String::from_utf8_lossy(&out.stdout);
	// Machine output never carries colour, even when
	// forced. A regression that leaks an escape into `--format json`
	// would corrupt every CI consumer parsing the output; verify the
	// unforced (no `--ansi always`) path first, then assert the data.
	assert!(
		!stdout.contains('\u{1b}'),
		"JSON output must not carry escapes: {stdout:?}"
	);
	let v: serde_json::Value = serde_json::from_str(&stdout).expect("JSON parses");
	let arr = v
		.get("findings")
		.and_then(|f| f.as_array())
		.expect("`findings` array");
	// Each service is expected to fire at least one finding; json-list
	// must reflect them all.
	assert!(arr.len() >= 3, "expected at least 3 findings, got {arr:?}");
	let services: Vec<&str> = arr
		.iter()
		.map(|f| f.get("service").and_then(|s| s.as_str()).unwrap_or("?"))
		.collect();
	for needed in ["pr", "caps", "secret"] {
		assert!(
			services.contains(&needed),
			"missing finding for `{needed}` in {arr:?}"
		);
	}
	// Per-object shape: every entry must carry all three keys, with
	// non-empty strings. A consumer reading `reason: ""` as "no finding"
	// would mis-classify an empty row, so we pin the absence.
	for entry in arr {
		for key in ["service", "check", "reason"] {
			let s = entry.get(key).and_then(|v| v.as_str()).unwrap_or("");
			assert!(
				!s.is_empty(),
				"every finding must carry non-empty `{key}`: {entry:?}"
			);
		}
	}
}

// ---------------------------------------------------------------------------
// secret_in_environment: the verdict must not depend on the caller's shell.
//
// A gate that flips on whether the operator exported a variable is not a
// gate. The same compose file audited with the variable exported and
// without it must give the same exit code and the same findings.
// ---------------------------------------------------------------------------

/// Like [`run`] but lets the test pin a single extra environment variable
/// (`SECRETO`) to a known value. Strips whatever the test runner happens
/// to have exported under `SECRETO` so the first run really is "unset".
fn run_with_secret_env(args: &[&str], secret_value: Option<&str>) -> Output {
	let mut cmd = Command::new(bin());
	cmd.args(args);
	for key in [
		"PODUP_LIBPOD_POOL",
		// `PODUP_LIBCOD_POOL` is the legacy typo'd spelling; the runtime
		// still reads it as a fallback so a developer's exported value
		// would otherwise leak into the spawned binary and silently
		// override its default pool size.
		"PODUP_LIBCOD_POOL",
		"PODMAN_SOCKET",
		"DOCKER_HOST",
		"COMPOSE_PROJECT_NAME",
		"COMPOSE_PROFILES",
		"COMPOSE_FILE",
		"NO_COLOR",
		// The variable under test. The test runner may have it exported
		// by accident; strip it so the "unset" run is genuinely unset.
		"SECRETO",
	] {
		cmd.env_remove(key);
	}
	if let Some(v) = secret_value {
		cmd.env("SECRETO", v);
	}
	cmd.output().expect("run podup audit")
}

#[test]
fn audit_secret_in_environment_verdict_is_independent_of_var_export() {
	// Same compose audited with `SECRETO` exported and without it must
	// give the same exit code and the same findings. The risk is the
	// same either way: a secret ends up in the container's environment
	// whether it was authored as a literal or interpolated from
	// `${VAR}`. A gate that flips on whether the developer's shell
	// happened to export the variable is not a gate.
	let body = "services:\n  web:\n    image: alpine:3.20\n    environment:\n      - DB_PASSWORD=${SECRETO}\n";
	let path = write_compose(body);
	let p = path.to_str().unwrap();

	let out_unset = run_with_secret_env(&["-f", p, "audit", "--strict"], None);
	let out_set = run_with_secret_env(&["-f", p, "audit", "--strict"], Some("valor"));

	assert_eq!(
		out_unset.status.code(),
		out_set.status.code(),
		"exit codes must agree; unset: {:?}, set: {:?}\nstdout unset:\n{}\nstderr unset:\n{}\nstdout set:\n{}\nstderr set:\n{}",
		out_unset.status.code(),
		out_set.status.code(),
		String::from_utf8_lossy(&out_unset.stdout),
		String::from_utf8_lossy(&out_unset.stderr),
		String::from_utf8_lossy(&out_set.stdout),
		String::from_utf8_lossy(&out_set.stderr),
	);
	// Both runs flag the secret-bearing key.
	let stdout_unset = String::from_utf8_lossy(&out_unset.stdout);
	let stdout_set = String::from_utf8_lossy(&out_set.stdout);
	assert!(
		stdout_unset.contains("secret_in_environment"),
		"unset run must still flag the secret-bearing key:\n{stdout_unset}"
	);
	assert!(
		stdout_set.contains("secret_in_environment"),
		"set run must still flag the secret-bearing key:\n{stdout_set}"
	);
	// Neither run echoes the resolved value back into the message.
	// Whatever `SECRETO` happens to resolve to, the message names the
	// key, never the value.
	assert!(
		!stdout_unset.contains("valor") && !stdout_set.contains("valor"),
		"neither run may echo the secret value into the message"
	);
	// The message no longer claims the value is hard-coded: that wording
	// is false for the `${VAR}` shape that survives into the audit (the
	// resolved value lives in the operator's environment, not in the
	// compose file itself).
	assert!(
		!stdout_unset.contains("hard-coded") && !stdout_set.contains("hard-coded"),
		"the message must not say `hard-coded`; that wording is false for ${{VAR}}:\nunset: {stdout_unset}\nset: {stdout_set}"
	);
}

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

// ---------------------------------------------------------------------------
// `_FILE` suffix tests live in `tests/audit_exit_codes/file_suffix.rs`
// (`#[path]` include below). Same child-module pattern as the
// sensitive-bind and wildcard-bind rows: the helpers above
// (`write_compose`, `run`) are inherited via `use super::*;`.
// ---------------------------------------------------------------------------

/// Property over the surface: `audit --strict` on the same file gives the
/// same verdict whether or not the caller exported each candidate variable.
/// The example-based test above covers `SECRETO` (the variable the original
/// #1841 defect keyed on); this one sweeps a basket of variables, so the
/// next member of the family — a different check that reads the operator's
/// environment — is caught the same way.
///
/// Stated as an equality between the exit codes, with no judgement of which
/// verdict is right: a gate whose answer is wrong is still a gate, and one
/// that flips between runs is not a gate at all.
#[test]
fn audit_strict_verdict_is_independent_of_every_candidate_var() {
	let body = "services:\n  web:\n    image: alpine:3.20\n    environment:\n      - DB_PASSWORD=${PODUP_PROP_AUD_A}\n      - API_KEY=${PODUP_PROP_AUD_B}\n      - PASSTHROUGH=true\n";
	let path = write_compose(body);
	let p = path.to_str().unwrap();

	let baseline = run(&["-f", p, "audit", "--strict"]);
	let baseline_code = baseline.status.code();

	for (key, value) in [
		("PODUP_PROP_AUD_A", "valor-a"),
		("PODUP_PROP_AUD_B", "valor-b"),
		("PODUP_PROP_AUD_C", "valor-c"),
		// A variable the file does not reference at all: exporting it must
		// still not change the verdict, because the runner is not the file.
		("PODUP_PROP_AUD_D", "valor-d"),
	] {
		let mut cmd = Command::new(bin());
		cmd.args(["-f", p, "audit", "--strict"]);
		for remove in [
			"PODUP_LIBPOD_POOL",
			"PODUP_LIBCOD_POOL",
			"PODMAN_SOCKET",
			"DOCKER_HOST",
			"COMPOSE_PROJECT_NAME",
			"COMPOSE_PROFILES",
			"COMPOSE_FILE",
			"NO_COLOR",
			key,
		] {
			cmd.env_remove(remove);
		}
		cmd.env(key, value);
		let exported = cmd.output().expect("run podup audit with var exported");
		assert_eq!(
			exported.status.code(),
			baseline_code,
			"exporting {key}={value} changed the audit verdict from {baseline_code:?} to {:?}\nbaseline stdout:\n{}\nexported stdout:\n{}",
			exported.status.code(),
			String::from_utf8_lossy(&baseline.stdout),
			String::from_utf8_lossy(&exported.stdout),
		);
	}
}

// The `sensitive_bind_mount` rows, split out to keep this file under the
// line limit. A child module, so it drives the binary through the helpers
// above.
#[path = "audit_exit_codes/sensitive_bind.rs"]
mod sensitive_bind;

// A `_FILE` key is judged on the value interpolation resolves to, not on
// the `${VAR}` text: a variable holding a path stays silent, one holding
// anything else is flagged, and an unset one resolves to the empty
// string, which is not a path and is flagged too.
#[test]
fn audit_file_suffix_is_judged_on_the_interpolated_value() {
	let body = "services:\n  db:\n    image: postgres:18-alpine\n    environment:\n      - POSTGRES_PASSWORD_FILE=${SECRETO}\n";
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	for (value, flagged) in [
		(Some("/run/secrets/pg"), false),
		(Some("./secrets/pg"), false),
		(Some("hunter2-real-password"), true),
		(None, true),
	] {
		let out = run_with_secret_env(&["-f", p, "audit", "--format", "json"], value);
		let stdout = String::from_utf8_lossy(&out.stdout);
		assert_eq!(
			stdout.contains("secret_in_environment"),
			flagged,
			"POSTGRES_PASSWORD_FILE=${{SECRETO}} with SECRETO={value:?} must {} secret_in_environment: {stdout}",
			if flagged { "raise" } else { "not raise" },
		);
	}
}

// The `port_published_on_wildcard` rows (`#1881`). Same child-module
// pattern: the helpers above (`write_compose`, `run`) are reused so
// the file stays focused on the audit-exit-code contract.
#[path = "audit_exit_codes/wildcard_bind.rs"]
mod wildcard_bind;

// The `_FILE` suffix tests for `secret_in_environment`. Same child-module
// pattern as the sibling rows above.
#[path = "audit_exit_codes/file_suffix.rs"]
mod file_suffix;

// The five runtime-hardening checks from #1894 (`no_restart_policy`,
// `no_init`, `no_health_action`, `swap_unbounded`, `no_cpu_limit`).
// Same child-module pattern as the sibling rows above.
#[path = "audit_exit_codes/runtime_checks.rs"]
mod runtime_checks;
