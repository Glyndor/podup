#![no_main]

//! Fuzz the memory/CPU/duration parsers for panics and overflow. They cast
//! through f64 and must never panic or saturate silently on hostile input.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	if let Ok(text) = std::str::from_utf8(data) {
		let _ = podup::size::parse_memory(text);
		let _ = podup::size::parse_cpus(text);
		let _ = podup::size::parse_duration_secs(text);
		let _ = podup::size::parse_duration_nanos(text);
	}
});
