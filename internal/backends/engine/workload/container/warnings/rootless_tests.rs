use crate::compose::types::Service;

use crate::engine::container::rootless::rootless_caveat_warnings;

#[test]
fn no_caveat_warnings_for_plain_service() {
	assert!(rootless_caveat_warnings("web", &Service::default()).is_empty());
}

#[test]
fn warns_for_each_rootless_caveat_field() {
	let service = Service {
		privileged: Some(true),
		oom_kill_disable: Some(true),
		mem_swappiness: Some(10),
		cpu_rt_runtime: Some(1000),
		links: vec!["db".into()],
		external_links: vec!["legacy_db:db".into()],
		..Service::default()
	};
	let warnings = rootless_caveat_warnings("web", &service);
	let joined = warnings.join("\n");
	assert_eq!(warnings.len(), 3, "{joined}");
	for needle in ["privileged", "links", "external_links"] {
		assert!(joined.contains(needle), "missing warning for {needle}");
	}
	// The cgroup-dependent limits are judged against the host the daemon
	// reports, where the value is applied or dropped; warning here too would
	// say it twice, and "ignored" would be wrong on cgroups v1.
	for needle in ["oom_kill_disable", "mem_swappiness", "cpu_rt_runtime"] {
		assert!(!joined.contains(needle), "{needle} warned here: {joined}");
	}
}
