//! End-to-end coverage for the five runtime-hardening checks from #1894:
//! `no_restart_policy`, `no_init`, `no_health_action`, `swap_unbounded`,
//! `no_cpu_limit`. Drives the compiled `podup` binary so the parse-time
//! wiring, the registry gate, and the exit-code flip are exercised the
//! same way an operator runs it.
//!
//! One compose trip-wires a single new check at a time on an otherwise
//! hardened service, so `--strict` exits non-zero and the JSON output
//! (`--format json`) carries that id. Each row's only finding is the
//! one the test owns; the rest are silenced by the hardened scaffolding.

use super::*;

/// Run `audit` against `body` with `--strict --format json` and return
/// the parsed JSON plus the process exit status. Forces `--strict` so
/// the gate-flip property is part of every assertion.
fn runtime_audit_json_for(body: &str) -> (serde_json::Value, std::process::ExitStatus) {
	let path = write_compose(body);
	let p = path.to_str().unwrap();
	let out = run(&["-f", p, "audit", "--strict", "--format", "json"]);
	assert!(
		out.status.success() || out.status.code() == Some(1),
		"`audit --strict` exits 0 or 1; got {:?}\nstderr: {}\nstdout: {}\nbody: {}",
		out.status.code(),
		String::from_utf8_lossy(&out.stderr),
		String::from_utf8_lossy(&out.stdout),
		body,
	);
	let stdout = String::from_utf8_lossy(&out.stdout);
	let v: serde_json::Value = serde_json::from_str(&stdout).expect("audit --format json parses");
	(v, out.status)
}

/// Hardened base the runtime-check tests slot the one trip-wire into.
/// Each field that any trip-wire overrides is left OUT of the base, and
/// the helper that wants to override it inserts the override plus the
/// hardened default. This keeps every hardened key present exactly
/// once in the final YAML, which the compose parser rejects otherwise.
fn hardened_with_override(
	restart: &str,
	init: &str,
	mem_limit: &str,
	memswap_limit: &str,
	cpus: &str,
	healthcheck_on_failure: Option<&str>,
) -> String {
	let hc = match healthcheck_on_failure {
		Some(action) => format!(
			"healthcheck:\n      test: [\"CMD\", \"true\"]\n      x-podman-on-failure: {action}"
		),
		None => "healthcheck:\n      test: [\"CMD\", \"true\"]".to_string(),
	};
	format!(
		"services:\n  db:\n    \
		 image: nginx:1.27@sha256:0e7bb5afc7e5e22ee46c4f2cd4a8b3fa63ad3f5d5e5e5e5e5e5e5e5e5e5e5e5e5e\n    \
		 read_only: true\n    \
		 cap_drop: [ALL]\n    \
		 security_opt: [no-new-privileges:true]\n    \
		 pids_limit: 200\n    \
		 mem_limit: {mem_limit}\n    \
		 memswap_limit: {memswap_limit}\n    \
		 init: {init}\n    \
		 restart: {restart}\n    \
		 cpus: {cpus}\n    \
		 {hc}\n    \
		 userns_mode: auto\n"
	)
}

/// A clean hardened service: every field present, no override.
fn hardened_clean() -> String {
	hardened_with_override(
		"unless-stopped",
		"true",
		"512m",
		"512m",
		"\"1\"",
		Some("restart"),
	)
}

/// Whether `findings` contains an entry for `service` whose `check` id
/// equals `id`.
fn has_runtime_finding(findings: &[serde_json::Value], id: &str) -> bool {
	findings
		.iter()
		.any(|f| f.get("check").and_then(|c| c.as_str()) == Some(id))
}

/// The hardened base passes `--strict` cleanly: every check the
/// scaffolding covers is silenced by the keys it carries. A regression
/// that drops any of the five new runtime keys flips this test red.
#[test]
fn runtime_hardened_clean_passes_strict() {
	let (v, status) = runtime_audit_json_for(&hardened_clean());
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	for id in [
		"no_restart_policy",
		"no_init",
		"no_health_action",
		"swap_unbounded",
		"no_cpu_limit",
	] {
		assert!(
			!has_runtime_finding(findings, id),
			"hardened service must not fire `{id}`: {findings:?}"
		);
	}
	assert_eq!(
		status.code(),
		Some(0),
		"hardened service must pass --strict: {findings:?}"
	);
}

/// `no_restart_policy`: a hardened service missing only `restart:` and
/// `deploy.restart_policy:`. `--strict` exits 1 and the JSON carries
/// the id. The override passes a literal `null`, which the YAML parser
/// turns into `None`, the same as an absent key.
#[test]
fn runtime_no_restart_policy_fires_when_missing() {
	let body = hardened_with_override("~", "true", "512m", "512m", "\"1\"", Some("restart"));
	let (v, status) = runtime_audit_json_for(&body);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		has_runtime_finding(findings, "no_restart_policy"),
		"`no_restart_policy` must fire: {findings:?}"
	);
	assert_eq!(
		status.code(),
		Some(1),
		"--strict exits 1 on missing restart: {findings:?}"
	);
}

/// `no_init`: a hardened service with `init: false`. `--strict` exits
/// 1 and the JSON carries the id.
#[test]
fn runtime_no_init_fires_when_false() {
	let body = hardened_with_override(
		"unless-stopped",
		"false",
		"512m",
		"512m",
		"\"1\"",
		Some("restart"),
	);
	let (v, status) = runtime_audit_json_for(&body);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		has_runtime_finding(findings, "no_init"),
		"`no_init` must fire: {findings:?}"
	);
	assert_eq!(
		status.code(),
		Some(1),
		"--strict exits 1 on init: false: {findings:?}"
	);
}

/// `no_health_action`: a hardened service with a non-disabled healthcheck
/// but no `x-podman-on-failure`. The override's healthcheck action is
/// `None`, which the helper turns into a healthcheck with a test and
/// no extension. `--strict` exits 1 and the JSON carries the id.
#[test]
fn runtime_no_health_action_fires_when_healthcheck_has_no_action() {
	let body = hardened_with_override("unless-stopped", "true", "512m", "512m", "\"1\"", None);
	let (v, status) = runtime_audit_json_for(&body);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		has_runtime_finding(findings, "no_health_action"),
		"`no_health_action` must fire: {findings:?}"
	);
	assert_eq!(
		status.code(),
		Some(1),
		"--strict exits 1 on healthcheck without action: {findings:?}"
	);
}

/// `swap_unbounded`: a hardened service with `memswap_limit: 512m`
/// while `mem_limit: 256m`. `--strict` exits 1 and the JSON carries
/// the id.
#[test]
fn runtime_swap_unbounded_fires_when_memswap_exceeds_mem_limit() {
	let body = hardened_with_override(
		"unless-stopped",
		"true",
		"256m",
		"512m",
		"\"1\"",
		Some("restart"),
	);
	let (v, status) = runtime_audit_json_for(&body);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		has_runtime_finding(findings, "swap_unbounded"),
		"`swap_unbounded` must fire: {findings:?}"
	);
	assert_eq!(
		status.code(),
		Some(1),
		"--strict exits 1 on memswap != mem_limit: {findings:?}"
	);
}

/// `no_cpu_limit`: a hardened service with `cpus: not-a-number` (which
/// the engine silently drops, leaving no CPU limit). `--strict` exits
/// 1 and the JSON carries the id.
#[test]
fn runtime_no_cpu_limit_fires_when_cpus_unparseable() {
	let body = hardened_with_override(
		"unless-stopped",
		"true",
		"512m",
		"512m",
		"not-a-number",
		Some("restart"),
	);
	let (v, status) = runtime_audit_json_for(&body);
	let findings = v.get("findings").and_then(|f| f.as_array()).unwrap();
	assert!(
		has_runtime_finding(findings, "no_cpu_limit"),
		"`no_cpu_limit` must fire: {findings:?}"
	);
	assert_eq!(
		status.code(),
		Some(1),
		"--strict exits 1 on unparseable cpus: {findings:?}"
	);
}

/// `audit --list-checks` lists all five new check ids without a compose
/// file. The drift contract an integrator relies on: an id the listing
/// names is one a finding can emit.
#[test]
fn runtime_list_checks_lists_the_five_new_ids() {
	let dir = std::env::temp_dir().join(format!(
		"podup-runtime-list-{}-{}",
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
	let listed: std::collections::BTreeSet<&str> = arr
		.iter()
		.filter_map(|e| e.get("id").and_then(|s| s.as_str()))
		.collect();
	for id in [
		"no_restart_policy",
		"no_init",
		"no_health_action",
		"swap_unbounded",
		"no_cpu_limit",
	] {
		assert!(
			listed.contains(id),
			"listing missing `{id}`: stdout={stdout}"
		);
	}
}
