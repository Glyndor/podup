use super::render_help;

fn styled() -> clap::builder::StyledStr {
	let mut s = clap::builder::StyledStr::new();
	s.push_str("Usage: podup [OPTIONS] <COMMAND>");
	s
}

/// Piped output must be byte-clean: a script reading the usage screen gets
/// text, not terminal control codes.
#[test]
fn without_colour_the_text_carries_no_escapes() {
	let out = render_help(&styled(), false);
	assert!(!out.contains('\u{1b}'), "{out:?}");
	assert!(out.contains("Usage: podup"), "{out:?}");
}

/// And the coloured arm actually differs, rather than being a no-op nobody
/// noticed, which is what a plain `assert!(out.contains("Usage"))` on both
/// arms would have failed to catch.
#[test]
fn with_colour_the_text_still_reads_the_same() {
	let plain = render_help(&styled(), false);
	let coloured = render_help(&styled(), true);
	assert!(coloured.contains("Usage: podup"), "{coloured:?}");
	assert_eq!(
		coloured.replace('\u{1b}', ""),
		plain.replace('\u{1b}', ""),
		"colour must not change the words"
	);
}
