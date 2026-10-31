//! `port_published_on_wildcard` and `port_published_on_all_interfaces`
//! describe different risks, and a merge regression would let one drop the
//! other's finding.
//!
//! Split out of `tests/cli/audit/audit_exit_codes/wildcard_bind/mod.rs`; the helpers
//! are inherited via `use super::*`.

use super::*;

/// `5432:5432` fires `port_published_on_all_interfaces` (no host IP,
/// every interface). Without `--wildcard-binds` it does NOT raise
/// `port_published_on_wildcard`; with the flag it still does not
/// raise it. The two checks are disjoint on the same mapping, so a
/// merge regression would either drop the all-interfaces finding or
/// surface a wildcard finding on a mapping that has no wildcard
/// (`#1881`).
#[test]
fn wildcard_does_not_fire_on_host_ip_less_short_form_without_flag() {
	let (v, status) = audit_json_for(&hardened_with_port("\"5432:5432\""), &[]);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		!wildcard_match(findings, "db", ""),
		"5432:5432 must not raise port_published_on_wildcard without --wildcard-binds: {findings:?}"
	);
	assert!(
		findings
			.iter()
			.any(|f| f.get("check").and_then(|c| c.as_str())
				== Some("port_published_on_all_interfaces")),
		"5432:5432 must still raise port_published_on_all_interfaces: {findings:?}"
	);
	assert_eq!(
		status.code(),
		Some(1),
		"--strict exits 1 on 5432:5432: {findings:?}"
	);
}

/// `5432:5432` and `0.0.0.0:5432:5432` in the same compose file: the
/// all-interfaces check fires once (for `5432:5432`) and the
/// wildcard check fires once (for `0.0.0.0:5432:5432`). A regression
/// that merged the two would either drop one or surface two of
/// either kind; the file shape catches both.
#[test]
fn wildcard_and_all_interfaces_are_disjoint_on_same_file() {
	let body = "services:\n  db:\n    \
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
		 - \"5432:5432\"\n      \
		 - \"0.0.0.0:6379:6379\"\n";
	let (v, status) = audit_json_for(body, &["--wildcard-binds"]);
	assert_eq!(
		status.code(),
		Some(1),
		"--strict exits 1 with both findings: {status:?}"
	);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	let wildcard_hits: Vec<&str> = findings
		.iter()
		.filter(|f| f.get("check").and_then(|c| c.as_str()) == Some("port_published_on_wildcard"))
		.map(|f| f.get("reason").and_then(|r| r.as_str()).unwrap_or(""))
		.collect();
	let all_hits: Vec<&str> = findings
		.iter()
		.filter(|f| {
			f.get("check").and_then(|c| c.as_str()) == Some("port_published_on_all_interfaces")
		})
		.map(|f| f.get("reason").and_then(|r| r.as_str()).unwrap_or(""))
		.collect();
	assert_eq!(
		wildcard_hits.len(),
		1,
		"one wildcard finding: {wildcard_hits:?}"
	);
	assert_eq!(
		all_hits.len(),
		1,
		"one all-interfaces finding: {all_hits:?}"
	);
	assert!(
		wildcard_hits[0].contains("6379"),
		"wildcard finding must name the 6379 mapping: {}",
		wildcard_hits[0]
	);
	assert!(
		all_hits[0].contains("5432"),
		"all-interfaces finding must name the 5432 mapping: {}",
		all_hits[0]
	);
	assert!(
		!wildcard_hits[0].contains("5432"),
		"wildcard finding must NOT cover 5432: {}",
		wildcard_hits[0]
	);
	assert!(
		!all_hits[0].contains("6379"),
		"all-interfaces finding must NOT cover 6379: {}",
		all_hits[0]
	);
}
