//! The audit verdict must not depend on the operator's shell.
//!
//! A gate that flips on whether the operator exported a variable is not a
//! gate. The same compose file audited with a variable exported and without it
//! must give the same exit code and the same findings.
//!
//! Split out from `tests/audit/audit_exit_codes.rs` so the parent file stays under
//! the repository line limit; the helpers (`run`, `run_with_secret_env`,
//! `write_compose`, `bin`) are inherited via `use super::*`.

use super::*;

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
