//! `--no-interpolate` prints the compose file without evaluating
//! placeholders, including the required-var form that would otherwise fail.
//!
//! Split out from `tests/cli/surface/cli_diagnostics.rs`; the helpers (`bin`,
//! `compose_file`) are inherited via `use super::*`.

use super::*;

#[test]
fn config_no_interpolate_keeps_placeholders_literal() {
	let (_dir, compose) = compose_file(
		"docker-compose.yml",
		"services:\n  web:\n    image: \"alpine:${PODUP_TAG}\"\n",
	);
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
fn config_no_interpolate_skips_required_var_error() {
	// `--no-interpolate` must not evaluate a required-var `${VAR:?msg}`: with the
	// variable unset the command should still succeed and print the placeholder
	// literally, rather than failing on the required-var check.
	let (_dir, compose) = compose_file(
		"docker-compose.yml",
		"services:\n  web:\n    image: ${MUST_SET:?required}\n",
	);
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
