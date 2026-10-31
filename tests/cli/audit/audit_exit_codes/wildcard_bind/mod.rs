//! End-to-end coverage for the `port_published_on_wildcard` check and
//! the `--wildcard-binds` opt-in flag (`#1881`). Drives the compiled
//! `podup` binary so the parse-time wiring, the registry gate, and
//! the exit-code flip are exercised the same way an operator runs it.
//!
//! Split out of `tests/cli/audit/audit_exit_codes/wildcard_bind/mod.rs` so each file stays
//! under the repository line limit; this file holds the helpers and the `mod`
//! lines, and the scenario files inherit them with `use super::*`.

use super::*;

/// Run `audit` with optional `--wildcard-binds` against `body` and
/// return both the parsed JSON and the process exit status. Forces
/// `--strict` so the gate-flip property is part of every assertion
/// (a missing finding flips the exit code to 1, the property CI
/// consumers depend on).
fn audit_json_for(body: &str, args: &[&str]) -> (serde_json::Value, std::process::ExitStatus) {
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	let mut full = vec!["-f", p, "audit", "--strict", "--format", "json"];
	full.extend_from_slice(args);
	let out = run(&full);
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
/// equals `port_published_on_wildcard` and whose `reason` contains
/// `needle`.
fn wildcard_match(findings: &[serde_json::Value], service: &str, needle: &str) -> bool {
	findings.iter().any(|f| {
		f.get("service").and_then(|s| s.as_str()) == Some(service)
			&& f.get("check").and_then(|c| c.as_str()) == Some("port_published_on_wildcard")
			&& f.get("reason")
				.and_then(|r| r.as_str())
				.is_some_and(|r| r.contains(needle))
	})
}

/// Hardened base the wildcard tests slot ports into. Every check that
/// fires on a bare `image: nginx` is silenced here, so the only
/// `--strict` exit-1 finding is the one the test owns (the wildcard
/// finding or the all-interfaces finding).
fn hardened_with_port(port: &str) -> String {
	format!(
		"services:\n  db:\n    \
		 image: nginx:1.27@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e\n    \
		 read_only: true\n    \
		 cap_drop: [ALL]\n    \
		 security_opt: [no-new-privileges:true]\n    \
		 pids_limit: 200\n    \
		 mem_limit: 512m\n    \
		 memswap_limit: 512m\n    \
		 init: true\n    \
		 restart: unless-stopped\n    \
		 cpus: \"1\"\n    \
		 healthcheck:\n      \
		 test: [\"CMD\", \"true\"]\n      \
		 x-podman-on-failure: restart\n    \
		 userns_mode: auto\n    \
		 ports:\n      \
		 - {port}\n"
	)
}

// Which port mappings the check matches once `--wildcard-binds` is set.
mod wildcard_matches;

// The wildcard check and the all-interfaces check stay disjoint.
mod all_interfaces;

// The opt-in flag is discoverable from `--list-checks --format json`.
mod list_checks;
