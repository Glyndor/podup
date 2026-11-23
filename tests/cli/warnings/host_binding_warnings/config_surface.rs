//! The `config` surface: every active host-binding mode produces its own
//! warning line, and `--no-warn` (a flag meant for `up`/`create`/`run`) does
//! not silence them.
//!
//! Split out from `tests/cli/warnings/host_binding_warnings.rs`; the helpers (`bin`,
//! `compose_file`) are inherited via `use super::*`.

use super::*;

/// A compose file that declares every host-binding mode the detector
/// knows about, one per service, so a test can grep stderr for each
/// warning by service name without false positives.
const EVERY_MODE_COMPOSE: &str = "\
services:
  net:
    image: alpine:3.19
    network_mode: host
  priv:
    image: alpine:3.19
    privileged: true
  pid:
    image: alpine:3.19
    pid: host
  ipc:
    image: alpine:3.19
    ipc: host
  uts:
    image: alpine:3.19
    uts: host
  cgroup:
    image: alpine:3.19
    cgroup: host
  userns:
    image: alpine:3.19
    userns_mode: host
  shared_net:
    image: alpine:3.19
    network_mode: \"container:sidecar\"
  shared_pid:
    image: alpine:3.19
    pid: \"container:sidecar\"
  plain:
    image: alpine:3.19
";

#[test]
fn config_surfaces_every_active_host_binding_mode() {
	let (_dir, file) = compose_file("compose.yaml", EVERY_MODE_COMPOSE);
	let out = Command::new(bin())
		.arg("-f")
		.arg(&file)
		.arg("config")
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup config");

	assert!(
		out.status.success(),
		"config must exit cleanly: {:?}",
		out.stderr
	);
	let stderr = String::from_utf8_lossy(&out.stderr);

	// Each declared mode produces its own `podup: warning:` line at default log
	// level: the `config` surface is independent of `--no-warn`, so an
	// operator who never runs `up` still sees the modes in CI logs.
	for (service, field) in [
		("net", "network_mode: host"),
		("priv", "privileged: true"),
		("pid", "pid: host"),
		("ipc", "ipc: host"),
		("uts", "uts: host"),
		("cgroup", "cgroup: host"),
		("userns", "userns_mode: host"),
		("shared_net", "network_mode: container:sidecar"),
		("shared_pid", "pid: container:sidecar"),
	] {
		assert!(
			stderr.contains(&format!("service \"{service}\"")) && stderr.contains(field),
			"config must warn on {service}: {field}; got stderr:\n{stderr}"
		);
	}

	// The plain service must NOT warn; pin that the detector does not
	// over-trigger.
	let plain_warning = stderr
		.lines()
		.find(|l| l.contains("podup: warning:") && l.contains("service \"plain\""));
	assert!(
		plain_warning.is_none(),
		"plain service triggered a warning: {plain_warning:?}"
	);

	// stdout stays a clean YAML pipe for `config`.
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		!stdout.contains("podup: warning:"),
		"diagnostics must stay on stderr; got stdout:\n{stdout}"
	);
}

#[test]
fn config_warning_is_unaffected_by_no_warn() {
	// A host-binding mode is a security warning, so `--no-warn` does not
	// silence it on `config`, as it does not on `up`.
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: alpine:3.19\n    network_mode: host\n",
	);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "--no-warn", "config"])
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup --no-warn config");
	assert!(
		out.status.success(),
		"{}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("network_mode: host"),
		"--no-warn must not silence config: {stderr}"
	);
}

#[test]
fn config_with_no_active_modes_is_silent() {
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: alpine:3.19\n",
	);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config"])
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup config");
	assert!(
		out.status.success(),
		"{}",
		String::from_utf8_lossy(&out.stderr)
	);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains("shares the host's"),
		"a clean service triggered a host-binding warning: {stderr}"
	);
}

#[test]
fn no_warn_keeps_the_host_binding_warnings_at_config_parse() {
	// `config` goes through the same detector as `up`, and `--no-warn` does not
	// silence a security warning on either: a file with a host mode still
	// warns under `config --no-warn`.
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: alpine:3.19\n    network_mode: host\n",
	);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "--no-warn", "config"])
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup --no-warn config");
	assert!(out.status.success());
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("network_mode: host"),
		"--no-warn must NOT silence config: {stderr}"
	);
}

#[test]
fn unknown_no_warn_after_subcommand_is_accepted() {
	// `--no-warn` is a global flag, so it is equally valid before and after
	// the subcommand. Parsing must succeed (a clap failure would exit
	// non-zero before the warning code ever runs).
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: alpine:3.19\n    network_mode: host\n",
	);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "config", "--no-warn"])
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup config --no-warn");
	assert!(
		out.status.success(),
		"{}",
		String::from_utf8_lossy(&out.stderr)
	);
}
