//! `sensitive_bind_mount`: one integration test per row of the list.
//!
//! The unit suite already pins the verdict for each path in isolation; these
//! files drive the compiled binary so the JSON shape and exit code also cover
//! the check end to end. One test per row of the prefix list, one per exact
//! socket match, and three shapes asserted rather than implied: a
//! non-sensitive mount does not fire, `:ro` still fires, and a path the
//! operator chose on purpose (the project directory) does not fire.
//!
//! Split out of `tests/cli/audit/audit_exit_codes/sensitive_bind/mod.rs` so each file stays
//! under the repository line limit; this file holds the helpers and the `mod`
//! lines, and the scenario files inherit them with `use super::*`.

use super::*;

/// Run `audit` against `body` and return the JSON output as a parsed
/// value. Forces `--strict` so a missing finding flips the exit code
/// to 1, the property CI consumers depend on.
fn audit_json_for(body: &str) -> (serde_json::Value, std::process::ExitStatus) {
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--strict", "--format", "json"]);
	assert!(
		out.status.success() || out.status.code() == Some(1),
		"`audit --strict` exits 0 or 1; got {:?}\nstderr: {}\nstdout: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	let v: serde_json::Value = serde_json::from_str(&stdout).expect("audit --format json parses");
	(v, out.status)
}

/// Whether `findings` contains an entry for `service` whose `check` id
/// equals `sensitive_bind_mount` and whose `reason` contains `needle`.
fn sensitive_match(findings: &[serde_json::Value], service: &str, needle: &str) -> bool {
	findings.iter().any(|f| {
		f.get("service").and_then(|s| s.as_str()) == Some(service)
			&& f.get("check").and_then(|c| c.as_str()) == Some("sensitive_bind_mount")
			&& f.get("reason")
				.and_then(|r| r.as_str())
				.is_some_and(|r| r.contains(needle))
	})
}

// The rows matched by directory prefix, including the runtime directories
// that expose a socket without naming it.
mod directory_prefixes;

// The rows matched by an exact socket path.
mod socket_paths;

// The host root entry: an exact match, not a prefix, in every spelling.
mod host_root;

// `:ro` discloses as much as `:rw`; the access mode never clears a finding.
mod access_mode;

// Mounts the check must leave alone on purpose.
mod negative_mounts;
