#![no_main]

//! Fuzz the dotenv parser (quote/escape handling, inline comments, multi-line
//! values) for panics on malformed `.env` content.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	if let Ok(text) = std::str::from_utf8(data) {
		let _ = podup::fuzz_api::dotenv_parse(text);
	}
});
