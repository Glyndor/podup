//! Hostile HTTP framing, seen from `podup ps`.
//!
//! The first request every command sends is `_ping`, so a hostile answer to it
//! reaches the same client code as a hostile answer to anything else. The
//! later cases let the ping through and attack the container listing, which is
//! parsed rather than streamed.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::run::{assert_refused, assert_survived, podup};
use super::server::{self, flood, ping_ok, reply, Request};

const BUDGET: Duration = Duration::from_secs(60);
const MIB: usize = 1024 * 1024;
/// More than any cap, and more than the ceiling on resident memory twice over.
const BIG: usize = 400 * MIB;

/// Run `podup ps` against a fake whose `_ping` answer is `hostile`.
fn ps_with_hostile_ping(what: &str, hostile: impl Fn(&mut UnixStream) + Send + Sync + 'static) {
	let fake = server::start(move |s, r: &Request| {
		if r.target.contains("_ping") {
			hostile(s)
		}
	});
	let home = tempfile::tempdir().unwrap();
	assert_refused(what, &podup(&fake.sock, &home, &["ps"], BUDGET));
}

/// Run `podup ps` against a fake that answers `_ping` honestly and every other
/// request with `hostile`.
fn ps_with_hostile_listing(
	what: &str,
	hostile: impl Fn(&mut UnixStream) + Send + Sync + 'static,
) -> super::run::Outcome {
	let fake = server::start(move |s, r: &Request| {
		if r.target.contains("_ping") {
			ping_ok(s)
		} else {
			hostile(s)
		}
	});
	let home = tempfile::tempdir().unwrap();
	let outcome = podup(&fake.sock, &home, &["ps"], BUDGET);
	assert_survived(what, &outcome);
	outcome
}

#[test]
fn headers_that_never_end_are_cut_off() {
	ps_with_hostile_ping("endless headers", |s| {
		let _ = s.write_all(b"HTTP/1.1 200 OK\r\n");
		flood(
			s,
			b"X-Pad: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\r\n",
			256 * MIB,
		);
	});
}

#[test]
fn one_header_line_that_never_ends_is_cut_off() {
	ps_with_hostile_ping("endless header value", |s| {
		let _ = s.write_all(b"HTTP/1.1 200 OK\r\nX-Pad: ");
		flood(s, &[b'a'; 4096], 256 * MIB);
	});
}

#[test]
fn a_negative_content_length_is_refused() {
	ps_with_hostile_ping("Content-Length: -1", |s| {
		let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\nOK");
	});
}

#[test]
fn a_content_length_past_u64_is_refused() {
	ps_with_hostile_ping("Content-Length past u64", |s| {
		let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999999\r\n\r\n");
	});
}

#[test]
fn two_conflicting_content_lengths_are_refused() {
	ps_with_hostile_ping("conflicting Content-Length", |s| {
		let _ = s.write_all(
			b"HTTP/1.1 200 OK\r\nLibpod-API-Version: 5.0.0\r\nContent-Length: 2\r\n\
			  Content-Length: 40\r\n\r\nOK",
		);
	});
}

#[test]
fn a_malformed_chunked_body_is_refused() {
	ps_with_hostile_ping("chunk size is not hex", |s| {
		let _ = s.write_all(
			b"HTTP/1.1 200 OK\r\nLibpod-API-Version: 5.0.0\r\n\
			  Transfer-Encoding: chunked\r\n\r\nZZZZ\r\nOK\r\n0\r\n\r\n",
		);
	});
}

#[test]
fn a_chunk_size_that_overflows_is_refused() {
	ps_with_hostile_ping("chunk size overflows u64", |s| {
		let _ = s.write_all(
			b"HTTP/1.1 200 OK\r\nLibpod-API-Version: 5.0.0\r\n\
			  Transfer-Encoding: chunked\r\n\r\nFFFFFFFFFFFFFFFFFFFF\r\nOK\r\n",
		);
	});
}

#[test]
fn a_body_shorter_than_its_content_length_is_refused() {
	ps_with_hostile_ping("body cut short", |s| {
		let _ = s.write_all(
			b"HTTP/1.1 200 OK\r\nLibpod-API-Version: 5.0.0\r\nContent-Length: 1000000\r\n\r\nOK",
		);
	});
}

#[test]
fn a_hangup_in_the_middle_of_the_status_line_is_refused() {
	ps_with_hostile_ping("cut status line", |s| {
		let _ = s.write_all(b"HTTP/1.1 20");
	});
}

#[test]
fn bytes_that_are_not_http_are_refused() {
	ps_with_hostile_ping("not HTTP", |s| {
		let _ = s.write_all(&[
			0x00, 0xff, 0xfe, b'\r', b'\n', 0x80, 0x81, b'\r', b'\n', b'\r', b'\n',
		]);
	});
}

#[test]
fn an_endless_run_of_100_continue_is_refused() {
	ps_with_hostile_ping("100 Continue forever", |s| {
		flood(s, b"HTTP/1.1 100 Continue\r\n\r\n", 64 * MIB);
	});
}

#[test]
fn a_redirect_is_an_error_not_a_destination() {
	ps_with_hostile_ping("redirect", |s| {
		reply(s, 301, "Location: http://203.0.113.9/steal\r\n", b"");
	});
}

#[test]
fn a_ping_without_a_version_is_refused() {
	let fake = server::start(|s, r: &Request| {
		if r.target.contains("_ping") {
			reply(s, 200, "", b"OK")
		}
	});
	let home = tempfile::tempdir().unwrap();
	let o = podup(&fake.sock, &home, &["ps"], BUDGET);
	assert_refused("no Libpod-API-Version", &o);
	assert!(o.text.contains("requires Podman"), "{}", o.text);
}

#[test]
fn a_ping_body_past_the_response_cap_is_cut_off() {
	// 400 MiB promised, 64 MiB is the cap: the rest must never be buffered.
	ps_with_hostile_ping("400 MiB ping body", |s| {
		let head = format!(
			"HTTP/1.1 200 OK\r\nLibpod-API-Version: 5.0.0\r\nContent-Length: {BIG}\r\n\r\n"
		);
		let _ = s.write_all(head.as_bytes());
		flood(s, &[b'a'; 65536], BIG);
	});
}

#[test]
fn an_error_body_past_the_response_cap_is_cut_off() {
	ps_with_hostile_ping("400 MiB error body", |s| {
		let _ = s.write_all(format!("HTTP/1.1 500 X\r\nContent-Length: {BIG}\r\n\r\n").as_bytes());
		flood(s, &[b'a'; 65536], BIG);
	});
}

#[test]
fn a_daemon_that_never_answers_is_given_up_on() {
	// Slow-loris: the connection opens and nothing ever comes back. The client
	// owns a 120 s read timeout; the budget leaves room for it and no more.
	let fake = server::start(|s, _| server::stall(s));
	let home = tempfile::tempdir().unwrap();
	let outcome = podup(&fake.sock, &home, &["ps"], Duration::from_secs(150));
	assert_refused("silent daemon", &outcome);
	assert!(
		outcome.took < Duration::from_secs(140),
		"{:?}",
		outcome.took
	);
}

#[test]
fn control_an_honest_daemon_gets_an_honest_answer() {
	let o = ps_with_hostile_listing("honest listing", |s| reply(s, 200, "", b"[]"));
	assert_eq!(o.code, Some(0), "{}", o.text);
}

#[test]
fn a_listing_that_stops_halfway_is_refused() {
	let o = ps_with_hostile_listing("truncated JSON", |s| {
		reply(
			s,
			200,
			"",
			br#"[{"Id":"abc","Names":["/hostile-web-1"],"Ima"#,
		);
	});
	assert_ne!(o.code, Some(0), "{}", o.text);
}

#[test]
fn a_million_levels_in_a_field_podup_does_not_read_are_skipped_safely() {
	// serde_json skips an unknown field without recursing, so this is accepted
	// rather than refused: what matters is that it is no stack overflow and no
	// memory growth. The depth limit itself is pinned on the event feed, whose
	// records are parsed into a value.
	let body = format!("[{{\"Zzz\":{}{}}}]", "[".repeat(MIB), "]".repeat(MIB));
	let o = ps_with_hostile_listing("1M-deep unread field", move |s| {
		reply(s, 200, "", body.as_bytes())
	});
	assert_eq!(o.code, Some(0), "{}", o.text);
}

#[test]
fn a_listing_of_the_wrong_shape_is_refused() {
	for body in [
		&b"{}"[..],
		b"\"x\"",
		b"[1,2,3]",
		b"[{\"Names\":7}]",
		b"\xff\xfe",
	] {
		let o = ps_with_hostile_listing("wrong JSON shape", move |s| reply(s, 200, "", body));
		assert_ne!(o.code, Some(0), "{body:?}: {}", o.text);
	}
}

#[test]
fn a_listing_past_the_response_cap_is_cut_off() {
	let o = ps_with_hostile_listing("400 MiB listing", |s| {
		let _ =
			s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {BIG}\r\n\r\n[").as_bytes());
		flood(
			s,
			b"\"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",",
			BIG,
		);
	});
	assert_ne!(o.code, Some(0), "{}", o.text);
}
