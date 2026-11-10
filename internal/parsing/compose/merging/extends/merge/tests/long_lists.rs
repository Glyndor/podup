use super::*;
use std::time::{Duration, Instant};

/// How many entries each side of the merge carries. Compared pairwise, two
/// lists of this size are over a billion string comparisons; merged by key
/// they are a fraction of a second.
const ENTRIES: usize = 20_000;

fn merged(base: &str, over: &str) -> crate::compose::types::Service {
	let base = parse_str(base).unwrap();
	let over_file = parse_str(over).unwrap();
	let raw: serde_yaml::Value = serde_yaml::from_str(over).unwrap();
	let directives = crate::compose::tags::collect(&raw);
	crate::compose::extends::merge_service_tagged(
		base.services["web"].clone(),
		over_file.services["web"].clone(),
		directives.get("web"),
	)
}

fn service_with(key: &str, items: impl Iterator<Item = String>) -> String {
	let mut text = format!("services:\n  web:\n    image: x\n    {key}:\n");
	for item in items {
		text.push_str(&format!("      - \"{item}\"\n"));
	}
	text
}

fn within_the_budget(started: Instant) {
	assert!(
		started.elapsed() < Duration::from_secs(10),
		"merging two lists of {ENTRIES} entries took {:?}",
		started.elapsed()
	);
}

#[test]
fn two_long_dns_lists_merge_without_rescanning() {
	let base = service_with("dns", (0..ENTRIES).map(|i| format!("a{i}.example")));
	// Half of the override repeats the base, half is new.
	let over = service_with(
		"dns",
		(ENTRIES / 2..ENTRIES + ENTRIES / 2).map(|i| format!("a{i}.example")),
	);
	let started = Instant::now();
	let out = merged(&base, &over);
	within_the_budget(started);
	let list = out.dns.to_list();
	assert_eq!(list.len(), ENTRIES + ENTRIES / 2);
	assert_eq!(list[0], "a0.example");
	assert_eq!(list[ENTRIES], format!("a{ENTRIES}.example"));
}

#[test]
fn two_long_volume_lists_merge_by_target_without_rescanning() {
	let base = service_with("volumes", (0..ENTRIES).map(|i| format!("/h{i}:/c{i}")));
	// The override remaps the first half to another host path and adds new targets.
	let over = service_with(
		"volumes",
		(0..ENTRIES + ENTRIES / 2).map(|i| format!("/other{i}:/c{i}")),
	);
	let started = Instant::now();
	let out = merged(&base, &over);
	within_the_budget(started);
	assert_eq!(out.volumes.len(), ENTRIES + ENTRIES / 2);
	let first = serde_yaml::to_string(&out.volumes[0]).unwrap();
	assert!(
		first.contains("/other0"),
		"the override wins at a shared target: {first}"
	);
}

#[test]
fn two_long_lists_of_a_serialised_kind_merge_without_rescanning() {
	let base = service_with(
		"ports",
		(0..ENTRIES).map(|i| format!("{}:{}", 1000 + i % 60000, i)),
	);
	let over = service_with(
		"ports",
		(0..ENTRIES).map(|i| format!("{}:{}", 1000 + i % 60000, i)),
	);
	let started = Instant::now();
	let out = merged(&base, &over);
	within_the_budget(started);
	assert_eq!(out.ports.len(), ENTRIES, "exact duplicates are dropped");
}
