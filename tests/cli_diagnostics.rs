//! CLI-level checks for diagnostic surfacing: forward-compat warnings must be
//! visible with no `RUST_LOG` set, must go to stderr, and must carry the
//! `podup:` program prefix — while stdout stays a clean YAML pipe for `config`.

use std::fs;
use std::process::Command;

fn bin() -> &'static str {
	env!("CARGO_BIN_EXE_podup")
}

/// Write a compose file with an unsupported key into a fresh temp dir and
/// return its path. The unknown key drives the forward-compat diagnostic.
fn compose_with_unknown_key() -> std::path::PathBuf {
	let dir = std::env::temp_dir().join(format!("podup-diag-{}", std::process::id()));
	fs::create_dir_all(&dir).expect("create temp dir");
	let path = dir.join("compose.yaml");
	fs::write(
		&path,
		"services:\n  web:\n    image: nginx:1.27\n    bogus_field: oops\n",
	)
	.expect("write compose file");
	path
}

#[test]
fn config_warns_on_stderr_with_clean_stdout_and_no_rust_log() {
	let file = compose_with_unknown_key();
	let output = Command::new(bin())
		.arg("-f")
		.arg(&file)
		.arg("config")
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup config");

	assert!(output.status.success(), "config exits cleanly");

	let stdout = String::from_utf8_lossy(&output.stdout);
	let stderr = String::from_utf8_lossy(&output.stderr);

	// The diagnostic fires by default (no RUST_LOG) and lands on stderr with
	// the unified `podup: warning:` prefix.
	assert!(
		stderr.contains("podup: warning:"),
		"warning prefixed and on stderr; got stderr:\n{stderr}"
	);
	assert!(
		stderr.contains("bogus_field"),
		"warning names the unknown key; got stderr:\n{stderr}"
	);

	// stdout is the resolved YAML only — no diagnostics interleaved.
	assert!(stdout.contains("services:"), "stdout carries the YAML");
	assert!(
		!stdout.contains("warning"),
		"stdout stays a clean pipe; got stdout:\n{stdout}"
	);

	let _ = fs::remove_dir_all(file.parent().unwrap());
}

#[test]
fn completions_emit_a_script_to_stdout() {
	let output = Command::new(bin())
		.args(["completions", "bash"])
		.output()
		.expect("run podup completions bash");
	assert!(output.status.success(), "completions exits cleanly");
	let stdout = String::from_utf8_lossy(&output.stdout);
	assert!(
		stdout.contains("_podup") || stdout.contains("complete"),
		"emits a bash completion script; got:\n{}",
		&stdout[..stdout.len().min(200)]
	);
}

/// A minimal valid two-service compose file in a fresh temp dir.
fn compose_two_services() -> std::path::PathBuf {
	let dir = std::env::temp_dir().join(format!("podup-cfg-{}", std::process::id()));
	fs::create_dir_all(&dir).expect("create temp dir");
	let path = dir.join("compose.yaml");
	fs::write(
		&path,
		"services:\n  web:\n    image: nginx:1.27\n  db:\n    image: postgres:16\n",
	)
	.expect("write compose file");
	path
}

#[test]
fn config_services_lists_service_names() {
	let file = compose_two_services();
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config", "--services"])
		.output()
		.expect("run config --services");
	assert!(out.status.success());
	let names: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
	assert!(
		names.contains(&"web") && names.contains(&"db"),
		"got: {names:?}"
	);
}

#[test]
fn config_format_json_emits_json() {
	let file = compose_two_services();
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config", "--format", "json"])
		.output()
		.expect("run config --format json");
	assert!(out.status.success());
	let stdout = String::from_utf8_lossy(&out.stdout);
	let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid JSON");
	assert!(parsed["services"]["web"].is_object());
}

#[test]
fn config_quiet_validates_without_output() {
	let valid = compose_two_services();
	let ok = Command::new(bin())
		.args(["-f", valid.to_str().unwrap(), "config", "-q"])
		.output()
		.unwrap();
	assert!(ok.status.success());
	assert!(ok.stdout.is_empty(), "quiet must print nothing");

	// A syntactically broken compose file fails validation (non-zero exit).
	let dir = std::env::temp_dir().join(format!("podup-cfgbad-{}", std::process::id()));
	fs::create_dir_all(&dir).unwrap();
	let badfile = dir.join("compose.yaml");
	fs::write(&badfile, "services:\n  web:\n    image: [unterminated\n").unwrap();
	let err = Command::new(bin())
		.args(["-f", badfile.to_str().unwrap(), "config", "-q"])
		.output()
		.unwrap();
	assert!(!err.status.success(), "invalid config must fail under -q");
}

#[test]
fn config_no_interpolate_keeps_placeholders_literal() {
	let dir = std::env::temp_dir().join(format!("podup-noint-{}", std::process::id()));
	fs::create_dir_all(&dir).unwrap();
	let compose = dir.join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: \"alpine:${PODUP_TAG}\"\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	// Default config interpolates ${PODUP_TAG} (unset → empty).
	let interp = Command::new(bin())
		.args(["-f", c, "config"])
		.env_remove("PODUP_TAG")
		.output()
		.unwrap();
	assert!(interp.status.success());
	assert!(
		!String::from_utf8_lossy(&interp.stdout).contains("${PODUP_TAG}"),
		"default config must interpolate the placeholder"
	);

	// --no-interpolate leaves it literal.
	let raw = Command::new(bin())
		.args(["-f", c, "config", "--no-interpolate"])
		.env_remove("PODUP_TAG")
		.output()
		.unwrap();
	assert!(
		raw.status.success(),
		"config --no-interpolate failed: {:?}",
		raw.stderr
	);
	assert!(
		String::from_utf8_lossy(&raw.stdout).contains("${PODUP_TAG}"),
		"--no-interpolate must keep ${{PODUP_TAG}} literal"
	);
}

#[test]
fn config_warns_on_unset_interpolation_variable() {
	// An unset `${VAR}` interpolates to an empty string but must warn on stderr
	// (matching docker compose) so a config typo does not pass silently.
	let dir = std::env::temp_dir().join(format!("podup-unset-warn-{}", std::process::id()));
	fs::create_dir_all(&dir).unwrap();
	let compose = dir.join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: ${PODUP_UNSET_IMAGE}\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let out = Command::new(bin())
		.args(["-f", c, "config"])
		.env_remove("PODUP_UNSET_IMAGE")
		.output()
		.unwrap();
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("PODUP_UNSET_IMAGE") && stderr.contains("not set"),
		"expected an unset-variable warning on stderr, got: {stderr:?}"
	);
}

#[test]
fn config_no_interpolate_skips_required_var_error() {
	// `--no-interpolate` must not evaluate a required-var `${VAR:?msg}`: with the
	// variable unset the command should still succeed and print the placeholder
	// literally, rather than failing on the required-var check.
	let dir = std::env::temp_dir().join(format!("podup-noint-req-{}", std::process::id()));
	fs::create_dir_all(&dir).unwrap();
	let compose = dir.join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: ${MUST_SET:?required}\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	// With interpolation on, the required-var error fails the command.
	let interp = Command::new(bin())
		.args(["-f", c, "config"])
		.env_remove("MUST_SET")
		.output()
		.unwrap();
	assert!(
		!interp.status.success(),
		"default config must fail on the required-var"
	);

	// With --no-interpolate, the file is printed uninterpolated and succeeds.
	let raw = Command::new(bin())
		.args(["-f", c, "config", "--no-interpolate"])
		.env_remove("MUST_SET")
		.output()
		.unwrap();
	assert!(
		raw.status.success(),
		"config --no-interpolate must not evaluate the required-var: {:?}",
		String::from_utf8_lossy(&raw.stderr)
	);
	assert!(
		String::from_utf8_lossy(&raw.stdout).contains("${MUST_SET:?required}"),
		"--no-interpolate must keep the placeholder literal"
	);
}
