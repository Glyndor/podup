#![no_main]

//! Fuzz variable substitution (`${VAR}`, `${VAR:-default}`, `${VAR:?err}`,
//! escapes) for panics, hangs, or unbounded expansion. The first input line
//! seeds a single variable so default/alternate branches are exercised both
//! set and unset.

use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;

fuzz_target!(|data: &[u8]| {
	let Ok(text) = std::str::from_utf8(data) else {
		return;
	};
	let mut vars = HashMap::new();
	// Seed one variable from the input so set-path modifiers are reached.
	if let Some((first, _)) = text.split_once('\n') {
		vars.insert("VAR".to_string(), first.to_string());
	}
	let _ = podup::substitute::substitute(text, &vars);
});
