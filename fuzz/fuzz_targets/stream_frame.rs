#![no_main]

//! Fuzz the libpod stream framer with raw daemon-controlled bytes: the 8-byte
//! multiplexed frame header (including hostile size fields) and the
//! newline-delimited JSON line splitter. Must never panic or index out of
//! bounds on malformed framing.

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
	// Multiplexed frame parsing over arbitrary bytes: drain every complete
	// frame the input contains, mirroring the streaming reassembly loop.
	let mut frame_buf = BytesMut::from(data);
	while podup::fuzz_api::parse_frame(&mut frame_buf).is_some() {}

	// Line splitter: drain every complete line the input contains.
	let mut buf = BytesMut::from(data);
	while podup::fuzz_api::take_json_line(&mut buf).is_some() {}
});
