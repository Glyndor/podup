//! The apt step of the Podman lane fails fast on a silent connection and stays
//! bounded.
//!
//! On 2026-11-06 the `Install qemu + cloud tools` step of the lane failed four
//! times in forty runs, each time with all three attempts ending at the 240 s
//! `timeout` and a log that said only `apt attempt N failed or timed out`: the
//! step ran `apt-get -qq`, so nothing named the host, and apt's own defaults wait
//! on a connection that has gone quiet. Every failure cost a rerun of a lane that
//! takes 40 to 100 minutes and held up the merge queue behind it.
//!
//! This pins what the step now says, for both workflows that carry it: the
//! connection options, a non-interactive frontend, a `timeout` on each apt call,
//! and a step budget that covers the attempts it makes and is not open ended.

use std::fs;

const WORKFLOWS: [&str; 2] = [
	".github/workflows/podman-lane.yml",
	".github/workflows/podman-lane-develop-nightly.yml",
];

/// The lines of the `Install qemu + cloud tools` step, without comments.
fn step(workflow: &str) -> Vec<String> {
	let text = fs::read_to_string(workflow).unwrap_or_else(|e| panic!("{workflow}: {e}"));
	let mut inside = false;
	let mut out = Vec::new();
	for line in text.lines() {
		let trimmed = line.trim_start();
		if trimmed.starts_with("- name: Install qemu + cloud tools") {
			inside = true;
			continue;
		}
		if inside && trimmed.starts_with("- name:") {
			break;
		}
		if inside && !trimmed.starts_with('#') && !trimmed.is_empty() {
			out.push(trimmed.to_string());
		}
	}
	assert!(!out.is_empty(), "{workflow}: the step is not there");
	out
}

fn number_after(lines: &[String], prefix: &str) -> u64 {
	lines
		.iter()
		.find_map(|l| l.strip_prefix(prefix))
		.and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
		.and_then(|n| n.parse().ok())
		.unwrap_or_else(|| panic!("no `{prefix}<number>` in {lines:?}"))
}

/// The number that follows `timeout ` on the line that runs `apt-get ... <verb>`.
fn timeout_of(lines: &[String], verb: &str) -> u64 {
	let line = lines
		.iter()
		.find(|l| l.contains("apt-get") && l.contains(&format!(" {verb}")))
		.unwrap_or_else(|| panic!("no apt-get {verb} in {lines:?}"));
	let at = line.find(verb).unwrap();
	let before = &line[..at];
	let tail = before.rsplit("timeout ").next().expect("a timeout");
	tail.split_whitespace()
		.next()
		.and_then(|n| n.parse().ok())
		.unwrap_or_else(|| panic!("`apt-get {verb}` is not under a timeout: {line}"))
}

#[test]
fn the_step_gives_up_on_a_silent_connection_and_is_not_silent_itself() {
	for workflow in WORKFLOWS {
		let lines = step(workflow);
		let joined = lines.join("\n");
		for needle in [
			"Acquire::Retries=",
			"Acquire::http::Timeout=",
			"Acquire::https::Timeout=",
			"DEBIAN_FRONTEND=noninteractive",
		] {
			assert!(
				joined.contains(needle),
				"{workflow}: missing {needle}\n{joined}"
			);
		}
		assert!(
			!joined.contains("-qq"),
			"{workflow}: -qq hides the host that holds the step\n{joined}"
		);
		// 20 seconds of silence is the limit of one connection.
		assert!(joined.contains("Acquire::http::Timeout=20"), "{workflow}");
	}
}

#[test]
fn every_apt_call_is_under_a_timeout_and_the_step_budget_covers_the_attempts() {
	for workflow in WORKFLOWS {
		let lines = step(workflow);
		let update = timeout_of(&lines, "update");
		let install = timeout_of(&lines, "install");
		let sleep = number_after(&lines, "sleep ");
		let attempts = lines
			.iter()
			.find_map(|l| l.strip_prefix("for attempt in "))
			.map(|rest| rest.trim_end_matches("; do").split_whitespace().count() as u64)
			.expect("a retry loop");
		let step_minutes = number_after(&lines, "timeout-minutes: ");
		let worst = attempts * (update + install + sleep);
		assert!(
			worst <= step_minutes * 60,
			"{workflow}: {attempts} attempts of {update}+{install}+{sleep} s need {worst} s, the step has {} s",
			step_minutes * 60
		);
		assert!(
			step_minutes <= 20,
			"{workflow}: the step must stay bounded, it has {step_minutes} minutes"
		);
		assert!(attempts >= 2, "{workflow}: one attempt is no retry");
	}
}
