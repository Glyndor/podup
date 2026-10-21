/// `window_size` answers `(rows, cols)`. Getting this backwards is invisible
/// on a square-ish terminal and mangles every line on a normal one.
#[test]
fn the_width_is_the_columns_not_the_rows() {
	assert_eq!(super::width_from(Some((30, 100))), Some(100));
	assert_eq!(super::width_from(None), None);
}

/// The live sink depends on the terminal alone. NO_COLOR=1 and `--ansi never`
/// collapse colour, not animation: a styled run that asked for no escapes
/// keeps its in-place repaint, and the styling is stripped at the renderer
/// instead of the renderer being skipped entirely (#1672).
///
/// Tested by reading both halves of the old decision separately so a future
/// regression that conflates them again has to fail both halves: `is_terminal`
/// is the only signal `live_terminal` consults; `stderr_colored` is consulted
/// only inside the renderer.
#[test]
fn live_sink_depends_on_the_terminal_not_on_colour() {
	// Read the body of `live_terminal` (without its leading doc comment) and
	// the body of `row::render`, so the assertions target the code, not the
	// documentation. A regression that re-conflates the two decisions fails
	// the body-of-`live_terminal` check; a regression that drops the renderer's
	// own colour gate fails the body-of-`row::render` check (and the
	// `no_color_keeps_the_spinner_and_drops_the_styling` test below).
	let terminal_src = include_str!("terminal.rs");
	let row_src = include_str!("row.rs");
	let live_body = fn_body(terminal_src, "fn live_terminal");
	let render_body = fn_body(row_src, "fn render");
	assert!(
		live_body.contains("is_terminal"),
		"live_terminal must read is_terminal: {live_body:?}"
	);
	assert!(
		!live_body.contains("stderr_colored"),
		"live_terminal must not consult stderr_colored: {live_body:?}"
	);
	assert!(
		render_body.contains("stderr_colored"),
		"render must consult stderr_colored so the styling tracks the choice: {render_body:?}"
	);
}

/// Slice `src` from `signature` to the next top-level `fn ` or end of file.
/// Doc comments and trailing whitespace are excluded so the assertion compares
/// only what `fn` actually executes.
fn fn_body(src: &str, signature: &str) -> String {
	let Some(start) = src.find(signature) else {
		return String::new();
	};
	let mut depth = 0;
	let mut end = src.len();
	let mut started = false;
	for (i, c) in src[start..].char_indices() {
		if c == '{' {
			depth += 1;
			started = true;
		} else if c == '}' {
			depth -= 1;
			if started && depth == 0 {
				end = start + i + 1;
				break;
			}
		}
	}
	src[start..end].to_string()
}
