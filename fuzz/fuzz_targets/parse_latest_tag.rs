#![no_main]

//! Fuzz the release-metadata parser that reads the `tag_name` out of the
//! GitHub "latest release" response. It must never panic: a body that is not
//! the expected JSON is refused, never accepted as a version.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	// A refusal is an ordinary outcome; what matters is that nothing panics.
	let _ = podup::fuzz_api::parse_latest_tag(data);
});
