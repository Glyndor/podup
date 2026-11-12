#![no_main]

//! Fuzz the two escapers that stand between external text (service names,
//! container names, table cells) and the terminal. Neither may panic; the
//! output must hold no control character at all, so no ESC, CR, NUL or C1
//! (U+0080 to U+009F) survives, and none of the bidirectional or zero-width
//! characters `is_terminal_unsafe` names; text that held none of them must come
//! back unchanged.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	let Ok(text) = std::str::from_utf8(data) else {
		return;
	};
	let (name, untouched) = podup::fuzz_api::sanitize_name(text);
	let cell = podup::ui::sanitize_cell(text);

	for (which, out) in [("sanitize_name", &name), ("sanitize_cell", &cell)] {
		assert!(
			!out.chars().any(char::is_control),
			"{which}({text:?}) kept a control character: {out:?}"
		);
		assert!(
			!out.chars()
				.any(|c| matches!(c, '\u{1b}' | '\r' | '\0' | '\u{80}'..='\u{9f}')),
			"{which}({text:?}) kept ESC, CR, NUL or a C1 character: {out:?}"
		);
	}
	assert_eq!(name, cell, "the two escapers disagree on {text:?}");
	assert!(
		!name.chars().any(podup::ui::is_terminal_unsafe),
		"sanitize_name({text:?}) kept a character that reorders or hides text: {name:?}"
	);

	// The escapers also hide the bidirectional marks and the zero-width characters
	// (#2428), so "needs escaping" is the predicate they use, not `is_control`.
	if text.chars().any(podup::ui::is_terminal_unsafe) {
		assert!(
			!untouched,
			"sanitize_name({text:?}) claims it borrowed the input"
		);
		assert_ne!(name, text);
	} else {
		assert!(untouched, "sanitize_name({text:?}) copied a safe input");
		assert_eq!(name, text, "safe text must pass through unchanged");
	}
	// Escaping once is enough: the output has nothing left to escape.
	assert_eq!(podup::fuzz_api::sanitize_name(&name).0, name);
	assert_eq!(podup::ui::sanitize_cell(&cell), cell);
});
