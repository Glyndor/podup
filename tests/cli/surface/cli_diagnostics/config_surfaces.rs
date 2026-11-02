//! The output surfaces of the `config` subcommand: the service list, the JSON
//! rendering, and the quiet validation that prints nothing.
//!
//! Split out from `tests/cli/surface/cli_diagnostics.rs`; the helpers (`bin`,
//! `compose_file`) are inherited via `use super::*`.

use super::*;

/// A minimal valid two-service compose file in a fresh temp dir.
fn compose_two_services() -> (TempDir, PathBuf) {
	compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: nginx:1.27\n  db:\n    image: postgres:16\n",
	)
}

#[test]
fn config_services_lists_service_names() {
	let (_dir, file) = compose_two_services();
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
	let (_dir, file) = compose_two_services();
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
	let (_dir, valid) = compose_two_services();
	let ok = Command::new(bin())
		.args(["-f", valid.to_str().unwrap(), "config", "-q"])
		.output()
		.unwrap();
	assert!(ok.status.success());
	assert!(ok.stdout.is_empty(), "quiet must print nothing");

	// A syntactically broken compose file fails validation (non-zero exit).
	let (_bad_dir, badfile) = compose_file(
		"compose.yaml",
		"services:\n  web:\n    image: [unterminated\n",
	);
	let err = Command::new(bin())
		.args(["-f", badfile.to_str().unwrap(), "config", "-q"])
		.output()
		.unwrap();
	assert!(!err.status.success(), "invalid config must fail under -q");
}

#[test]
fn ps_all_services_lists_the_file_without_a_podman_socket() {
	let (_dir, file) = compose_two_services();
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "ps", "--all-services"])
		.env("PODMAN_SOCKET", "/nonexistent/podman.sock")
		.output()
		.expect("run ps --all-services");
	assert!(
		out.status.success(),
		"{}",
		String::from_utf8_lossy(&out.stderr)
	);
	let names: Vec<&str> = std::str::from_utf8(&out.stdout).unwrap().lines().collect();
	assert_eq!(names, ["web", "db"]);
}

#[test]
fn ps_all_services_refuses_what_the_table_refuses() {
	let (_dir, file) = compose_two_services();
	for extra in [["--filter", "bogus=1"].as_slice(), ["nope"].as_slice()] {
		let out = Command::new(bin())
			.args(["-f", file.to_str().unwrap(), "ps", "--all-services"])
			.args(extra)
			.env("PODMAN_SOCKET", "/nonexistent/podman.sock")
			.output()
			.expect("run ps --all-services");
		assert!(!out.status.success(), "{extra:?} was accepted");
		assert!(out.stdout.is_empty(), "{extra:?} printed names");
	}
}

#[test]
fn ps_all_services_escapes_a_name_that_would_drive_the_terminal() {
	let (_dir, file) = compose_file(
		"compose.yaml",
		"services:\n  \"web\\u001b[2Jfake\": {image: alpine}\n",
	);
	let out = Command::new(bin())
		.args(["-f", file.to_str().unwrap(), "ps", "--all-services"])
		.env("PODMAN_SOCKET", "/nonexistent/podman.sock")
		.output()
		.expect("run ps --all-services");
	assert!(
		out.status.success(),
		"{}",
		String::from_utf8_lossy(&out.stderr)
	);
	assert!(!out.stdout.contains(&0x1b), "{:?}", out.stdout);
	assert!(String::from_utf8_lossy(&out.stdout).starts_with("web"));
}
