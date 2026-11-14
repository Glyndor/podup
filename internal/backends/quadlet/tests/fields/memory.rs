//! Memory and OOM keys that have no native Quadlet key: what the unit carries
//! through `PodmanArgs=`, and what it skips because Podman would refuse it.

use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

/// Generate service `s` of project `p` and return its unit and the warnings.
fn render(service_body: &str) -> (String, String) {
	let yaml = format!("services:\n  s:\n    image: app:1.0\n{service_body}");
	let file = parse_str(&yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let unit = unit_named(&out, "p-s.container").contents.clone();
	(unit, out.warnings.join("\n"))
}

#[test]
fn mem_reservation_reaches_podman_as_a_soft_limit() {
	let (unit, warnings) = render("    mem_reservation: 256m\n");
	assert!(
		unit.contains("PodmanArgs=--memory-reservation=\"256m\""),
		"{unit}"
	);
	assert!(!warnings.contains("mem_reservation"), "{warnings}");
}

#[test]
fn a_deploy_reservation_fills_in_for_a_missing_mem_reservation() {
	let (unit, _) =
		render("    deploy:\n      resources:\n        reservations:\n          memory: 128m\n");
	assert!(
		unit.contains("PodmanArgs=--memory-reservation=\"128m\""),
		"{unit}"
	);
}

#[test]
fn mem_reservation_wins_over_the_deploy_reservation() {
	let (unit, _) = render(
		"    mem_reservation: 256m\n    deploy:\n      resources:\n        reservations:\n          memory: 128m\n",
	);
	assert!(unit.contains("--memory-reservation=\"256m\""), "{unit}");
	assert!(!unit.contains("128m"), "{unit}");
}

#[test]
fn an_unlimited_reservation_adds_no_flag() {
	// Podman's CLI refuses a negative reservation, and no flag is already
	// no soft limit.
	let (unit, warnings) = render("    mem_reservation: -1\n");
	assert!(!unit.contains("--memory-reservation"), "{unit}");
	assert!(!warnings.contains("mem_reservation"), "{warnings}");
}

#[test]
fn memswap_limit_is_carried_next_to_a_memory_limit() {
	let (unit, warnings) = render("    mem_limit: 512m\n    memswap_limit: 1g\n");
	assert!(unit.contains("PodmanArgs=--memory-swap=\"1g\""), "{unit}");
	assert!(!warnings.contains("memswap_limit"), "{warnings}");
}

#[test]
fn memswap_limit_counts_a_deploy_memory_limit() {
	let (unit, _) = render(
		"    memswap_limit: 1g\n    deploy:\n      resources:\n        limits:\n          memory: 512m\n",
	);
	assert!(unit.contains("--memory-swap=\"1g\""), "{unit}");
}

#[test]
fn memswap_limit_without_a_memory_limit_is_skipped_and_reported() {
	// crun: "cannot set swap limit without the memory limit"; the unit would
	// never start.
	let (unit, warnings) = render("    memswap_limit: 1g\n");
	assert!(!unit.contains("--memory-swap"), "{unit}");
	assert!(
		warnings.contains("memswap_limit") && warnings.contains("mem_limit"),
		"{warnings}"
	);
}

#[test]
fn oom_score_adj_reaches_podman() {
	let (unit, warnings) = render("    oom_score_adj: -500\n");
	assert!(unit.contains("PodmanArgs=--oom-score-adj="), "{unit}");
	assert!(unit.contains("-500"), "{unit}");
	assert!(!warnings.contains("oom_score_adj"), "{warnings}");
}

#[test]
fn keys_cgroups_v2_refuses_stay_out_of_the_unit_and_say_why() {
	let (unit, warnings) = render("    mem_swappiness: 10\n    oom_kill_disable: true\n");
	assert!(!unit.contains("--memory-swappiness"), "{unit}");
	assert!(!unit.contains("--oom-kill-disable"), "{unit}");
	for key in ["mem_swappiness", "oom_kill_disable"] {
		assert!(
			warnings
				.lines()
				.any(|l| l.contains(key) && l.contains("cgroups v2")),
			"{key}: {warnings}"
		);
	}
}

#[test]
fn oom_kill_disable_false_is_quiet() {
	let (unit, warnings) = render("    oom_kill_disable: false\n");
	assert!(!unit.contains("oom-kill"), "{unit}");
	assert!(!warnings.contains("oom_kill_disable"), "{warnings}");
}
