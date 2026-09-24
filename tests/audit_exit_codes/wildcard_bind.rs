//! End-to-end coverage for the `port_published_on_wildcard` check and
//! the `--wildcard-binds` opt-in flag (`#1881`). Drives the compiled
//! `podup` binary so the parse-time wiring, the registry gate, and
//! the exit-code flip are exercised the same way an operator runs it.

use std::process::Output;

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

/// `--list-checks` lists the wildcard check id and the opt-in flag
/// in the JSON listing. The table shape is owned by
/// `tests/audit_list_checks.rs`; the redundant assertion here is on
/// the JSON shape a CI consumer pipes (`#1881`).
#[test]
fn wildcard_listed_in_list_checks_json_with_opt_in_flag() {
	let dir = std::env::temp_dir().join(format!(
		"podup-wildcard-list-{}-{}",
		std::process::id(),
		std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::Relaxed)
	));
	let _ = std::fs::remove_dir_all(&dir);
	std::fs::create_dir_all(&dir).unwrap();
	let out: Output = {
		let mut cmd = std::process::Command::new(bin());
		cmd.args(["audit", "--list-checks", "--format", "json"])
			.current_dir(&dir);
		for key in [
			"PODUP_LIBPOD_POOL",
			"PODUP_LIBCOD_POOL",
			"PODMAN_SOCKET",
			"DOCKER_HOST",
			"COMPOSE_PROJECT_NAME",
			"COMPOSE_PROFILES",
			"COMPOSE_FILE",
			"NO_COLOR",
		] {
			cmd.env_remove(key);
		}
		cmd.output().expect("audit --list-checks --format json")
	};
	let _ = std::fs::remove_dir_all(&dir);
	assert!(out.status.success(), "listing run failed: {:?}", out.status);
	let stdout = String::from_utf8_lossy(&out.stdout);
	let v: serde_json::Value = serde_json::from_str(&stdout).expect("JSON parses");
	let arr = v
		.get("checks")
		.and_then(|c| c.as_array())
		.expect("`checks` array");
	let wildcard = arr
		.iter()
		.find(|e| e.get("id").and_then(|s| s.as_str()) == Some("port_published_on_wildcard"))
		.expect("wildcard row must be listed");
	assert_eq!(
		wildcard.get("opt_in").and_then(|v| v.as_str()),
		Some("--wildcard-binds"),
		"wildcard row must carry `--wildcard-binds` in `opt_in`"
	);
}
