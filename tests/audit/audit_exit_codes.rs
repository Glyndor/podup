//! End-to-end checks for the `podup audit` subcommand's exit codes and JSON
//! shape. Drives the compiled `podup` binary against a tiny compose file on
//! disk so the parsing, profile honouring, and emit path are exercised the
//! same way an operator would; the unit suite already covers the checks
//! themselves in isolation.
//!
//! Names state the contract: `--strict` flips the exit code from 0
//! to 1 when any finding is present, and `--format json` is the
//! machine-readable surface CI consumes.
//!
//! This crate root holds the helpers every scenario file inherits through
//! `use super::*`, plus one `mod` line per scenario. The rows themselves
//! live in the files declared below.

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

// The exit code `--strict` produces, and the two shapes the human-readable
// table has to carry on the default run.
#[path = "audit_exit_codes/exit_codes.rs"]
mod exit_codes;

// `--format json`: every finding listed, stable schema, no escapes.
#[path = "audit_exit_codes/json_shape.rs"]
mod json_shape;

// Which keys fire `secret_in_environment` and which stay silent.
#[path = "audit_exit_codes/secret_environment.rs"]
mod secret_environment;

// The verdict must not depend on which variables the caller's shell
// happened to export.
#[path = "audit_exit_codes/env_independence.rs"]
mod env_independence;

// The `_FILE` suffix tests live in `tests/audit/audit_exit_codes/file_suffix.rs`.
// Same child-module pattern as the rows above: the helpers above
// (`write_compose`, `run`, `run_with_secret_env`) are inherited via
// `use super::*;`.
#[path = "audit_exit_codes/file_suffix.rs"]
mod file_suffix;

// The `sensitive_bind_mount` rows, split out to keep this file under the
// line limit. A child module, so it drives the binary through the helpers
// above.
#[path = "audit_exit_codes/sensitive_bind/mod.rs"]
mod sensitive_bind;

// The `port_published_on_wildcard` rows (`#1881`). Same child-module
// pattern: the helpers above (`write_compose`, `run`) are reused so
// the file stays focused on the audit-exit-code contract.
#[path = "audit_exit_codes/wildcard_bind/mod.rs"]
mod wildcard_bind;

// The five runtime-hardening checks from #1894 (`no_restart_policy`,
// `no_init`, `no_health_action`, `swap_unbounded`, `no_cpu_limit`).
// Same child-module pattern as the sibling rows above.
#[path = "audit_exit_codes/runtime_checks.rs"]
mod runtime_checks;
