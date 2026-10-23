//! Which port mappings `port_published_on_wildcard` matches once
//! `--wildcard-binds` is set, in short form and long form, and which
//! specific bindings it must leave alone.
//!
//! Split out of `tests/audit_exit_codes/wildcard_bind/mod.rs`; the helpers
//! are inherited via `use super::*`.

use super::*;

/// `0.0.0.0:5432:5432` (IPv4 wildcard) raises the wildcard check
/// only when the flag is set; without the flag it stays silent.
/// `--strict` exits 1 in the with-flag run and 0 in the
/// without-flag run, so a CI that adopts the flag today sees a gate
/// flip only on the same file it was already shipping.
#[test]
fn wildcard_fires_on_ipv4_wildcard_with_flag_only() {
	let body = hardened_with_port("\"0.0.0.0:5432:5432\"");
	let (v_no_flag, status_no_flag) = audit_json_for(&body, &[]);
	let findings_no_flag = v_no_flag
		.get("findings")
		.and_then(|f| f.as_array())
		.unwrap();
	assert!(
		!wildcard_match(findings_no_flag, "db", ""),
		"without --wildcard-binds the wildcard check must stay silent: {findings_no_flag:?}"
	);
	assert_eq!(
		status_no_flag.code(),
		Some(0),
		"without --wildcard-binds, --strict exits 0 on a wildcard port: {findings_no_flag:?}"
	);

	let (v_flag, status_flag) = audit_json_for(&body, &["--wildcard-binds"]);
	let findings_flag = v_flag.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		wildcard_match(findings_flag, "db", "5432"),
		"with --wildcard-binds the wildcard check must fire on 0.0.0.0:5432:5432: {findings_flag:?}"
	);
	assert_eq!(
		status_flag.code(),
		Some(1),
		"--wildcard-binds --strict exits 1 on a wildcard port: {findings_flag:?}"
	);
}

/// `[::]:5432:5432` (IPv6 wildcard): same shape as the IPv4 case.
#[test]
fn wildcard_fires_on_ipv6_wildcard_with_flag_only() {
	let body = hardened_with_port("\"[::]:5432:5432\"");
	let (v_no_flag, status_no_flag) = audit_json_for(&body, &[]);
	let findings_no_flag = v_no_flag
		.get("findings")
		.and_then(|f| f.as_array())
		.unwrap();
	assert!(
		!wildcard_match(findings_no_flag, "db", ""),
		"without --wildcard-binds the IPv6 wildcard must stay silent: {findings_no_flag:?}"
	);
	assert_eq!(
		status_no_flag.code(),
		Some(0),
		"without --wildcard-binds, --strict exits 0 on an IPv6 wildcard port: {findings_no_flag:?}"
	);

	let (v_flag, status_flag) = audit_json_for(&body, &["--wildcard-binds"]);
	let findings_flag = v_flag.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		wildcard_match(findings_flag, "db", "5432"),
		"with --wildcard-binds the IPv6 wildcard must fire: {findings_flag:?}"
	);
	assert_eq!(
		status_flag.code(),
		Some(1),
		"--wildcard-binds --strict exits 1 on an IPv6 wildcard port: {findings_flag:?}"
	);
}

/// `127.0.0.1:5432:5432` is a specific loopback, a deliberate
/// decision. With `--wildcard-binds` it does NOT raise
/// `port_published_on_wildcard`. A regression that fired on any
/// `ip:host:container` shape would slip past every "wildcard" test
/// above but flip this one.
#[test]
fn wildcard_does_not_fire_on_loopback_with_flag() {
	let (v, status) = audit_json_for(
		&hardened_with_port("\"127.0.0.1:5432:5432\""),
		&["--wildcard-binds"],
	);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		!wildcard_match(findings, "db", ""),
		"loopback must not fire wildcard even with --wildcard-binds: {findings:?}"
	);
	assert_eq!(
		status.code(),
		Some(0),
		"loopback with --wildcard-binds exits 0: {findings:?}"
	);
}

/// `192.168.1.10:5432:5432`: private LAN bind, a deliberate decision.
/// The wildcard check stays silent on it. With `--wildcard-binds`
/// the binary exits 0 (no wildcard finding, the all-interfaces check
/// also stays silent because there IS a host IP).
#[test]
fn wildcard_does_not_fire_on_private_lan_ip_with_flag() {
	let (v, status) = audit_json_for(
		&hardened_with_port("\"192.168.1.10:5432:5432\""),
		&["--wildcard-binds"],
	);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		!wildcard_match(findings, "db", ""),
		"private LAN must not fire wildcard: {findings:?}"
	);
	assert_eq!(
		status.code(),
		Some(0),
		"private LAN with --wildcard-binds exits 0: {findings:?}"
	);
}

/// Long-form `host_ip: "0.0.0.0"` and `host_ip: "::"` raise the
/// wildcard check. Pinning the long-form path keeps a regression
/// that only handled the short form from slipping past.
#[test]
fn wildcard_fires_on_long_form_wildcard_host_ip() {
	let ipv4 = "services:\n  db:\n    \
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
		 - target: 5432\n        \
		 published: 5432\n        \
		 host_ip: 0.0.0.0\n";
	let (v_v4, status_v4) = audit_json_for(ipv4, &["--wildcard-binds"]);
	let findings_v4 = v_v4.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		wildcard_match(findings_v4, "db", "5432"),
		"long-form host_ip 0.0.0.0 must fire wildcard: {findings_v4:?}"
	);
	assert_eq!(
		status_v4.code(),
		Some(1),
		"long-form 0.0.0.0 with --wildcard-binds exits 1"
	);

	let ipv6 = "services:\n  db:\n    \
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
		 - target: 5432\n        \
		 published: 5432\n        \
		 host_ip: \"::\"\n";
	let (v_v6, status_v6) = audit_json_for(ipv6, &["--wildcard-binds"]);
	let findings_v6 = v_v6.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		wildcard_match(findings_v6, "db", "5432"),
		"long-form host_ip :: must fire wildcard: {findings_v6:?}"
	);
	assert_eq!(
		status_v6.code(),
		Some(1),
		"long-form :: with --wildcard-binds exits 1"
	);
}
