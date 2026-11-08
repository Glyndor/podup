use super::*;

// --- the bound on the stop ------------------------------------------------

/// #1093: systemd bounds `ExecStop` independently of what podup does inside
/// it, at a 90s default. A stack whose slowest container needs longer stops
/// cleanly when a human runs `podup stop` and gets killed mid-stop at
/// reboot; the difference only appears during an unattended shutdown,
/// which is the worst version of it.
#[test]
fn render_service_unit_bounds_stop_above_the_longest_grace_period() {
	let mut o = opts_single();
	o.max_stop_grace_secs = Some(120);
	let s = render_service_unit(&o);
	assert!(
		s.contains("TimeoutStopSec=150"),
		"expected the longest grace plus headroom:\n{s}"
	);
}

/// Headroom, not the exact value: the stop has per-container teardown around
/// it, so a bound equal to the grace period would cut the last container off
/// just as it finishes.
#[test]
fn stop_timeout_leaves_headroom_over_the_grace_period() {
	let mut o = opts_single();
	o.max_stop_grace_secs = Some(10);
	let s = render_service_unit(&o);
	let line = s
		.lines()
		.find(|l| l.starts_with("TimeoutStopSec="))
		.expect("the key is present");
	let secs: u64 = line.trim_start_matches("TimeoutStopSec=").parse().unwrap();
	assert!(secs > 10, "must exceed the grace period, got {secs}");
}

/// No service asking for anything unusual leaves the key off entirely, so
/// systemd keeps its own default rather than podup restating it.
#[test]
fn no_grace_period_emits_no_stop_timeout() {
	let s = render_service_unit(&opts_single());
	assert!(!s.contains("TimeoutStopSec"), "{s}");
}
