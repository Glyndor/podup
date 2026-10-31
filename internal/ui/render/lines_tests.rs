use super::*;

/// The verb bands are what colour a progress line as it closes. A row that
/// closes with `Failed` previously fell through to the default green, the same
/// default that paints `Created`, so a failed row read as a successful one
/// except for the word. The fix is one prefix on the existing red arm (#1347).
#[test]
fn a_failed_progress_verb_is_red() {
	let red = Style::new().fg_color(Some(AnsiColor::Red.into()));
	let failed = action_style("Failed");
	let failed_lower = action_style("failed");
	assert_eq!(
		failed.render().to_string(),
		red.render().to_string(),
		"Failed must share the red band"
	);
	assert_eq!(
		failed_lower.render().to_string(),
		red.render().to_string(),
		"failed (lower) must share the red band"
	);
	// The verbs that were already red stay red; the change is additive.
	assert_eq!(
		action_style("Removed").render().to_string(),
		red.render().to_string()
	);
}

#[test]
fn progress_toggle_is_observable() {
	// Off by default-or-restored; toggling flips the observable state. Restore
	// afterwards so the process-global flag does not leak into other tests.
	let prev = progress_enabled();
	set_progress(false);
	assert!(!progress_enabled());
	set_progress(true);
	assert!(progress_enabled());
	set_progress(prev);
}
