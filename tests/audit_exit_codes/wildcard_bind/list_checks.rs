//! The opt-in flag is discoverable: `--list-checks --format json` names
//! both the check id and the flag that enables it.
//!
//! Split out of `tests/audit_exit_codes/wildcard_bind/mod.rs`; the helpers
//! are inherited via `use super::*`.

use std::process::Output;

use super::*;

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
