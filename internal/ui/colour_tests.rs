use super::*;

#[test]
fn colour_choice_resolution() {
	// Pure resolution: never touches the process-global choice, so it can't
	// race the production code (LinePrefixer, the table renderer) that reads it.
	temp_env::with_var_unset("NO_COLOR", || {
		assert!(!colored_with(ColorChoice::Never, true));
		assert!(colored_with(ColorChoice::Always, false));
		assert!(colored_with(ColorChoice::Auto, true));
		assert!(!colored_with(ColorChoice::Auto, false));
	});
	// NO_COLOR forces plain in Auto, regardless of the TTY.
	temp_env::with_var("NO_COLOR", Some("1"), || {
		assert!(!colored_with(ColorChoice::Auto, true));
		// ...but an explicit `always` still overrides NO_COLOR.
		assert!(colored_with(ColorChoice::Always, true));
	});
}
