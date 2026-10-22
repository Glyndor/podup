//! The `generate quadlet` surface: Quadlet has no Podman client to surface
//! warnings through, so the emitter runs at generate time, alongside the
//! emitted directives and gated by the same `--no-warn`.
//!
//! Split out from `tests/host_binding_warnings.rs`; the helpers (`bin`,
//! `compose_file`) are inherited via `use super::*`.

use super::*;

#[test]
fn generate_quadlet_warns_on_host_network_and_privileged() {
	// Quadlet has no Podman client to surface warnings through, so it must
	// emit them at generate time. The mode stays emitted in the unit file
	// (Network=host, PodmanArgs=--privileged); the warning is alongside, not
	// instead of, the mapping.
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  net:\n    image: alpine:3.19\n    network_mode: host\n  priv:\n    image: alpine:3.19\n    privileged: true\n  plain:\n    image: alpine:3.19\n",
	);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "generate", "quadlet"])
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup generate quadlet");
	assert!(out.status.success(), "{:?}", out.stderr);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("service \"net\"") && stderr.contains("network_mode: host"),
		"network_mode: host must warn at generate time; got stderr:\n{stderr}"
	);
	assert!(
		stderr.contains("service \"priv\"") && stderr.contains("privileged: true"),
		"privileged: true must warn at generate time; got stderr:\n{stderr}"
	);
	let plain = stderr
		.lines()
		.find(|l| l.contains("podup: warning:") && l.contains("service \"plain\""));
	assert!(
		plain.is_none(),
		"plain service triggered a warning: {plain:?}"
	);

	// Quadlet output (stdout) still carries the modes.
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		stdout.contains("Network=host"),
		"Network=host must appear in the generated unit; got:\n{stdout}"
	);
	assert!(
		stdout.contains("PodmanArgs=--privileged"),
		"PodmanArgs=--privileged must appear in the generated unit; got:\n{stdout}"
	);
}

#[test]
fn generate_quadlet_warns_on_container_namespace_sharing() {
	// `container:<id>` is the same isolation surface as `host` and podman
	// does not warn on it, so the Quadlet path must surface it instead.
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: alpine:3.19\n    network_mode: \"container:sidecar\"\n",
	);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "generate", "quadlet"])
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup generate quadlet");
	assert!(out.status.success(), "{:?}", out.stderr);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		stderr.contains("container:sidecar"),
		"network_mode: container:<id> must warn at generate time; got stderr:\n{stderr}"
	);
}

#[test]
fn generate_quadlet_no_warn_suppresses_the_warnings() {
	// `--no-warn` opts the operator out of the Quadlet-path warnings the
	// same way it does on the live engine. The mode is still emitted in the
	// unit file; only the per-mode `podup: warning:` line is suppressed.
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  net:\n    image: alpine:3.19\n    network_mode: host\n  priv:\n    image: alpine:3.19\n    privileged: true\n",
	);
	let out = Command::new(bin())
		.args([
			"-f",
			file.to_str().unwrap(),
			"--no-warn",
			"generate",
			"quadlet",
		])
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup --no-warn generate quadlet");
	assert!(out.status.success(), "{:?}", out.stderr);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		!stderr.contains("podup: warning: network_mode: host"),
		"--no-warn must silence the network_mode warning; got stderr:\n{stderr}"
	);
	assert!(
		!stderr.contains("podup: warning: privileged: true"),
		"--no-warn must silence the privileged warning; got stderr:\n{stderr}"
	);
	// The unit still carries the modes; suppression is the per-line warning,
	// not the emitted directive.
	let stdout = String::from_utf8_lossy(&out.stdout);
	assert!(
		stdout.contains("Network=host"),
		"Network=host must still be in the unit: {stdout}"
	);
	assert!(
		stdout.contains("PodmanArgs=--privileged"),
		"PodmanArgs=--privileged must still be in the unit: {stdout}"
	);
}

#[test]
fn generate_quadlet_no_warn_without_modes_is_silent() {
	// Sanity: `--no-warn` against a clean compose file produces no
	// host-binding / privilege-escalation warning. (The Quadlet path also
	// emits an unrelated platform advisory on non-Linux hosts, and that is not
	// gated on `--no-warn` and is intentionally left alone, so the assertion
	// filters to the host-binding lines only.)
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: alpine:3.19\n",
	);
	let out = Command::new(bin())
		.args([
			"-f",
			file.to_str().unwrap(),
			"--no-warn",
			"generate",
			"quadlet",
		])
		.env_remove("RUST_LOG")
		.output()
		.expect("run podup --no-warn generate quadlet");
	assert!(out.status.success(), "{:?}", out.stderr);
	let stderr = String::from_utf8_lossy(&out.stderr);
	for needle in [
		"network_mode: host",
		"privileged: true",
		"pid: host",
		"ipc: host",
		"uts: host",
		"cgroup: host",
		"userns_mode: host",
		"container:",
	] {
		assert!(
			!stderr.contains(needle),
			"--no-warn must silence the {needle} warning; got stderr:\n{stderr}"
		);
	}
}
