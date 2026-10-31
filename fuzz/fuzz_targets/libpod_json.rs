#![no_main]

//! Fuzz the JSON the Podman daemon sends back. Every response type the client
//! decodes is fed arbitrary bytes, among them deeply nested documents and
//! numbers far outside the field's type. A refusal is fine; a panic is not.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	// Every type on every input, so a corpus entry built for one still reaches
	// the others (the decoder takes its index modulo the number of types).
	for kind in 0..10 {
		let _ = podup::fuzz_api::libpod_json_decode(kind, data);
	}
});
