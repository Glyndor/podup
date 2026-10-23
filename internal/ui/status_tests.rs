use super::*;

/// The bug this exists to stop: `ls` reports `running(1), exited(1)`, and
/// styling it as one string let the first matching substring win: `exit`
/// came first, so a project with a service up rendered entirely red,
/// indistinguishable from one that is completely dead.
#[test]
fn a_mixed_project_is_not_painted_as_one_state() {
	let out = paint_status_cell("running(1), exited(1)");
	let (running, exited) = out.split_once(", ").expect("both segments survive");
	assert_ne!(
		running.replace("running(1)", ""),
		exited.replace("exited(1)", ""),
		"each state must carry its own colour: {out:?}"
	);
}

/// A container that ran to completion is not a failure. One-shot services
/// (migrations, seeds, a `command` that simply ends) live in this state.
#[test]
fn a_clean_exit_is_not_red() {
	let red = Style::new().fg_color(Some(AnsiColor::Red.into()));
	let clean = paint_status_cell("Exited (0)");
	assert!(
		!clean.contains(&red.render().to_string()),
		"a zero exit must not be red: {clean:?}"
	);
	let failed = paint_status_cell("Exited (7)");
	assert!(
		failed.contains(&red.render().to_string()),
		"a non-zero exit must stay red: {failed:?}"
	);
}

/// Digits after the first must not be mistaken for a clean exit.
#[test]
fn only_a_bare_zero_counts_as_clean() {
	assert!(is_clean_exit("exited (0)"));
	assert!(is_clean_exit("exited(0)"));
	assert!(!is_clean_exit("exited (10)"));
	assert!(!is_clean_exit("exited (07)"));
}

/// Padding is what keeps columns aligned, so colourising must not eat it.
#[test]
fn trailing_padding_survives_colourising() {
	let out = paint_status_cell("running   ");
	assert!(out.ends_with("   "), "{out:?}");
}

/// systemd's vocabulary reaches this through `autostart status`.
#[test]
fn systemd_states_are_coloured() {
	for word in ["active", "inactive", "failed", "not-found", "enabled"] {
		assert!(status_style(word).is_some(), "{word} should carry a colour");
	}
}

#[test]
fn status_style_is_semantic() {
	assert_ne!(status_style("running"), status_style("exited (1)"));
	assert_ne!(status_style("unhealthy"), status_style("healthy"));
	assert!(status_style("Up 2 minutes").is_some());
	assert!(status_style("paused").is_some());
	assert!(status_style("created").is_some());
	assert!(status_style("weird-state").is_none());
}
