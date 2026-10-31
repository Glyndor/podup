use super::*;

#[test]
fn warns_on_a_stop_grace_period_podup_cannot_read() {
	let msgs =
		diagnostics_for("services:\n  web:\n    image: nginx\n    stop_grace_period: 5 days\n");
	assert!(
		msgs.iter().any(|m| m.contains("stop_grace_period")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_a_restart_window_podup_cannot_read() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    deploy:\n      restart_policy:\n        \
		 window: soon\n",
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("deploy.restart_policy.window")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_a_healthcheck_interval_podup_cannot_read() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    healthcheck:\n      test: [\"CMD\", \
		 \"true\"]\n      interval: soon\n",
	);
	assert!(
		msgs.iter().any(|m| m.contains("healthcheck.interval")),
		"got: {msgs:?}"
	);
}

#[test]
fn the_warning_does_not_repeat_the_value() {
	let msgs =
		diagnostics_for("services:\n  web:\n    image: nginx\n    stop_grace_period: 5 days\n");
	for msg in &msgs {
		assert!(!msg.contains("5 days"), "the value is not repeated: {msg}");
	}
}

#[test]
fn a_duration_podup_can_read_is_not_warned_about() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    stop_grace_period: 1m30s\n    \
		 healthcheck:\n      test: [\"CMD\", \"true\"]\n      interval: 10s\n",
	);
	assert!(
		!msgs.iter().any(|m| m.contains("duration podup can read")),
		"unexpected diagnostics: {msgs:?}"
	);
}
