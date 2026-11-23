//! The parse-time port-exposure warning, and which commands surface it: the
//! write commands and `config` do, the read-only commands do not.
//!
//! Split out from `tests/cli/warnings/host_binding_warnings.rs`; the helpers (`bin`,
//! `compose_file`) are inherited via `use super::*`.

use super::*;

/// Compose file that publishes a port on every host interface (the `5432:5432`
/// short form), exercising the parse-time port-exposure detector.
const PORT_EXPOSED_COMPOSE: &str = "\
services:
  web:
    image: alpine:3.19
    ports:
      - \"5432:5432\"
";

/// Run `podup` against `args`, pointing it at a non-existent Podman socket so
/// the test does not require a daemon. The port-exposure warning is emitted
/// at parse time, before any API call, so the unreachable socket is enough to
/// prove the warning was already on (or absent from) stderr by the time the
/// socket error lands. Exit status is ignored: the test reads stderr.
fn run_podup_isolated(args: &[&str]) -> std::process::Output {
	Command::new(bin())
		.args(args)
		.env_remove("RUST_LOG")
		.env("PODMAN_SOCKET", "/nonexistent/podup-host-binding-test.sock")
		.output()
		.expect("run podup")
}

/// Substring that uniquely identifies the parse-time port-exposure warning
/// (the `is published on every interface` fragment is fixed text emitted by
/// `port_published_on_all_interfaces` in `internal/parsing/compose/diagnostics/port_exposure.rs`).
const PORT_EXPOSURE_NEEDLE: &str = "is published on every interface";

#[test]
fn up_emits_the_port_exposure_warning() {
	let (_dir, file) = compose_file("compose.yaml", PORT_EXPOSED_COMPOSE);
	let out = run_podup_isolated(&["-f", file.to_str().unwrap(), "up"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains(PORT_EXPOSURE_NEEDLE),
		"up must surface the port-exposure warning at parse time; got stderr:\n{stderr}"
	);
}

#[test]
fn up_no_warn_does_not_silence_the_port_exposure_warning() {
	let (_dir, file) = compose_file("compose.yaml", PORT_EXPOSED_COMPOSE);
	let out = run_podup_isolated(&["-f", file.to_str().unwrap(), "--no-warn", "up"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains(PORT_EXPOSURE_NEEDLE),
		"a port published on every interface is a security warning and --no-warn keeps it on up; got stderr:\n{stderr}"
	);
}

#[test]
fn ps_never_emits_the_port_exposure_warning() {
	let (_dir, file) = compose_file("compose.yaml", PORT_EXPOSED_COMPOSE);
	let out = run_podup_isolated(&["-f", file.to_str().unwrap(), "ps"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains(PORT_EXPOSURE_NEEDLE),
		"ps is read-only and must not surface the port-exposure warning; got stderr:\n{stderr}"
	);
}

#[test]
fn ps_no_warn_never_emits_the_port_exposure_warning() {
	let (_dir, file) = compose_file("compose.yaml", PORT_EXPOSED_COMPOSE);
	let out = run_podup_isolated(&["-f", file.to_str().unwrap(), "--no-warn", "ps"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains(PORT_EXPOSURE_NEEDLE),
		"--no-warn is moot on ps: the warning never fires; got stderr:\n{stderr}"
	);
}

#[test]
fn port_command_never_emits_the_port_exposure_warning() {
	let (_dir, file) = compose_file("compose.yaml", PORT_EXPOSED_COMPOSE);
	let out = run_podup_isolated(&["-f", file.to_str().unwrap(), "port", "web", "5432"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains(PORT_EXPOSURE_NEEDLE),
		"`port` is read-only and must not surface the port-exposure warning; got stderr:\n{stderr}"
	);
}

#[test]
fn logs_command_never_emits_the_port_exposure_warning() {
	let (_dir, file) = compose_file("compose.yaml", PORT_EXPOSED_COMPOSE);
	let out = run_podup_isolated(&["-f", file.to_str().unwrap(), "logs", "web"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains(PORT_EXPOSURE_NEEDLE),
		"`logs` is read-only and must not surface the port-exposure warning; got stderr:\n{stderr}"
	);
}

#[test]
fn top_command_never_emits_the_port_exposure_warning() {
	let (_dir, file) = compose_file("compose.yaml", PORT_EXPOSED_COMPOSE);
	let out = run_podup_isolated(&["-f", file.to_str().unwrap(), "top"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains(PORT_EXPOSURE_NEEDLE),
		"`top` is read-only and must not surface the port-exposure warning; got stderr:\n{stderr}"
	);
}

#[test]
fn config_emits_the_port_exposure_warning_regardless_of_no_warn() {
	let (_dir, file) = compose_file("compose.yaml", PORT_EXPOSED_COMPOSE);
	// A port published on every interface is a security warning, so
	// `--no-warn` does not silence it on `config` or on any other command.
	let without = run_podup_isolated(&["-f", file.to_str().unwrap(), "config"]);
	let without_stderr = String::from_utf8_lossy(&without.stderr);
	assert!(
		without_stderr.contains(PORT_EXPOSURE_NEEDLE),
		"config must surface the port-exposure warning; got stderr:\n{without_stderr}"
	);

	let with = run_podup_isolated(&["-f", file.to_str().unwrap(), "--no-warn", "config"]);
	let with_stderr = String::from_utf8_lossy(&with.stderr);
	assert!(
		with_stderr.contains(PORT_EXPOSURE_NEEDLE),
		"--no-warn must NOT silence config's port-exposure warning; got stderr:\n{with_stderr}"
	);
}
