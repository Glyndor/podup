//! Hostile streams: the NDJSON event feed and the multiplexed log framing.
//!
//! The deterministic counterpart of `fuzz/fuzz_targets/stream_frame.rs` and
//! `stream_ndjson.rs`: the fuzzers feed the parsers bytes, these feed the whole
//! command a daemon that lies, and read the exit code, the output and the peak
//! memory.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::run::{assert_refused, assert_survived, podup, Outcome};
use super::server::{self, flood, ping_ok, reply, Request};

const BUDGET: Duration = Duration::from_secs(60);
const MIB: usize = 1024 * 1024;
const CHUNKED: &[u8] = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n";
const LISTING: &[u8] =
	br#"[{"Id":"abc","Names":["/hostile-web-1"],"State":"running","Image":"alpine"}]"#;

/// One HTTP chunk carrying `data`.
fn chunk(s: &mut UnixStream, data: &[u8]) -> bool {
	s.write_all(format!("{:x}\r\n", data.len()).as_bytes())
		.is_ok()
		&& s.write_all(data).is_ok()
		&& s.write_all(b"\r\n").is_ok()
}

/// Send chunks of `data` until the client hangs up or `limit` bytes are out.
fn chunked_flood(s: &mut UnixStream, data: &[u8], limit: usize) {
	let _ = s.write_all(CHUNKED);
	let mut sent = 0;
	while sent < limit && chunk(s, data) {
		sent += data.len();
	}
}

/// `podup events` against a daemon whose feed is `feed`.
fn events(feed: impl Fn(&mut UnixStream) + Send + Sync + 'static) -> Outcome {
	let fake = server::start(move |s, r: &Request| {
		if r.target.contains("_ping") {
			ping_ok(s)
		} else {
			feed(s)
		}
	});
	let home = tempfile::tempdir().unwrap();
	podup(&fake.sock, &home, &["events"], BUDGET)
}

/// `podup logs` against a daemon whose log stream is `frames`.
fn logs(frames: impl Fn(&mut UnixStream) + Send + Sync + 'static) -> Outcome {
	let fake = server::start(move |s, r: &Request| {
		if r.target.contains("_ping") {
			ping_ok(s)
		} else if r.target.contains("/logs") {
			frames(s)
		} else {
			reply(s, 200, "", LISTING)
		}
	});
	let home = tempfile::tempdir().unwrap();
	podup(&fake.sock, &home, &["logs"], BUDGET)
}

/// An 8-byte multiplexed header: stream `kind`, payload length `len`.
fn header(kind: u8, len: u32) -> [u8; 8] {
	let n = len.to_be_bytes();
	[kind, 0, 0, 0, n[0], n[1], n[2], n[3]]
}

#[test]
fn an_event_line_that_never_ends_is_cut_off() {
	let o = events(|s| chunked_flood(s, &[b'a'; 65536], 256 * MIB));
	assert_refused("event line without a newline", &o);
	assert!(o.text.contains("stream-too-large"), "{}", o.text);
}

#[test]
fn an_event_line_nested_deeper_than_the_parser_allows_is_refused() {
	let line = format!("{}{}\n", "[".repeat(500_000), "]".repeat(500_000));
	let o = events(move |s| {
		let _ = s.write_all(CHUNKED);
		chunk(s, line.as_bytes());
	});
	assert_refused("500k-deep event", &o);
	assert!(o.text.contains("recursion limit"), "{}", o.text);
}

#[test]
fn an_endless_feed_of_events_does_not_grow_memory() {
	let o = events(|s| {
		let line = br#"{"Type":"container","Action":"start","Actor":{"Attributes":{"name":"x"}}}"#;
		let batch: Vec<u8> = (0..2000)
			.flat_map(|_| [&line[..], b"\n"].concat())
			.collect();
		chunked_flood(s, &batch, 32 * MIB);
	});
	assert_refused("32 MiB of events", &o);
}

#[test]
fn event_lines_that_are_not_json_are_refused() {
	for line in [
		&b"not json\n"[..],
		b"\xff\xfe\x00\n",
		b"{\"a\":\n",
		b"\x00\x00\x00\n",
	] {
		let o = events(move |s| {
			let _ = s.write_all(CHUNKED);
			chunk(s, line);
		});
		assert_survived("garbage event line", &o);
	}
}

#[test]
fn a_frame_announcing_four_gigabytes_is_refused() {
	let o = logs(|s| {
		let _ = s.write_all(CHUNKED);
		chunk(s, &[&header(1, u32::MAX)[..], b"x"].concat());
	});
	assert_refused("frame length 0xFFFFFFFF", &o);
	assert!(o.text.contains("stream-too-large"), "{}", o.text);
}

#[test]
fn a_frame_one_byte_over_the_cap_is_refused() {
	let o = logs(|s| {
		let _ = s.write_all(CHUNKED);
		chunk(s, &header(1, (MIB + 1) as u32));
	});
	assert_refused("frame length 1 MiB + 1", &o);
	assert!(o.text.contains("stream-too-large"), "{}", o.text);
}

#[test]
fn a_stream_of_empty_frames_does_not_spin_or_grow() {
	let o = logs(|s| chunked_flood(s, &header(1, 0).repeat(8192), 64 * MIB));
	assert_survived("zero-length frames", &o);
}

#[test]
fn frames_of_unknown_stream_types_are_skipped_not_buffered() {
	let mut frame = header(7, 16).to_vec();
	frame.extend_from_slice(&[b'z'; 16]);
	let o = logs(move |s| chunked_flood(s, &frame.repeat(4096), 64 * MIB));
	assert_survived("stream type 7", &o);
}

#[test]
fn a_frame_cut_in_half_is_not_a_clean_end() {
	let o = logs(|s| {
		let _ = s.write_all(CHUNKED);
		chunk(s, &[&header(1, 1000)[..], b"only a few bytes"].concat());
		let _ = s.write_all(b"0\r\n\r\n");
	});
	assert_refused("frame cut in half", &o);
}

/// The controls: with an honest daemon the same commands work, so the refusals
/// above come from the hostile bytes and from nothing else in the setup.
#[test]
fn control_an_honest_log_stream_is_printed() {
	let o = logs(|s| {
		let _ = s.write_all(CHUNKED);
		chunk(s, &[&header(1, 6)[..], b"hello\n"].concat());
		let _ = s.write_all(b"0\r\n\r\n");
	});
	assert_survived("honest logs", &o);
	assert!(o.text.contains("hello"), "{}", o.text);
}

#[test]
fn control_an_honest_event_is_printed() {
	let o = events(|s| {
		let _ = s.write_all(CHUNKED);
		chunk(s, b"{\"Type\":\"container\",\"Action\":\"start\"}\n");
	});
	assert!(o.text.contains("container"), "{}", o.text);
}

#[test]
fn raw_bytes_flooded_into_a_log_stream_stay_bounded() {
	let o = logs(|s| {
		let _ = s.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n");
		flood(s, &[0xAA; 65536], 512 * MIB);
	});
	assert_refused("512 MiB of unframed bytes", &o);
	assert!(o.text.contains("stream-too-large"), "{}", o.text);
}
