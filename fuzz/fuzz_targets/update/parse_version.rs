#![no_main]

//! Fuzz the release version parser the self-updater compares the latest tag
//! against. It must never panic: a malformed tag is refused, never mistaken
//! for a newer release.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	let Ok(text) = std::str::from_utf8(data) else {
		return;
	};
	// A refusal is an ordinary outcome; what matters is that nothing panics.
	let _ = podup::fuzz_api::parse_version(text);
});
