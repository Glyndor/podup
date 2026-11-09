#![no_main]

//! Fuzz the dotenv parser (quote/escape handling, inline comments, multi-line
//! values), including strict rejection and agreement on accepted inputs.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	if let Ok(text) = std::str::from_utf8(data) {
		let lenient = podup::fuzz_api::dotenv_parse(text);
		if let Ok(strict) = podup::fuzz_api::dotenv_parse_strict(text) {
			assert_eq!(strict, lenient);
		}
	}
});
