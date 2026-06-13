#![no_main]

//! Fuzz the full compose parse path: variable substitution, YAML
//! deserialization, anchor/alias merge-key handling, and type coercion. This is
//! podup's primary untrusted-input entry point.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	if let Ok(text) = std::str::from_utf8(data) {
		// Errors are expected for malformed input; we only care that the parser
		// never panics, hangs, or exhausts memory.
		let _ = podup::parse_str(text);
		let _ = podup::parse_str_raw(text);
	}
});
