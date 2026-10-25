//! What a hostile or broken release endpoint can do through `GitHubSource`.
//!
//! The fetch functions themselves need TLS, which no local test server can
//! speak to a client that only trusts the public roots. What can be reached is
//! everything after the bytes arrive (`read_capped`, `parse_latest_tag`) and the
//! refusal to use plaintext at all.

use super::*;

use std::io::Read;
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Yields `good` bytes and then fails the way a dropped connection does.
struct Dropped {
	good: usize,
}
impl Read for Dropped {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		if self.good == 0 {
			return Err(std::io::ErrorKind::ConnectionReset.into());
		}
		let n = self.good.min(buf.len());
		buf[..n].fill(b'x');
		self.good -= n;
		Ok(n)
	}
}

#[test]
fn a_download_cut_off_midway_is_an_error_not_a_short_asset() {
	let err = read_capped(Dropped { good: 4096 }, 1024 * 1024).unwrap_err();
	assert!(matches!(err, ComposeError::Io(_)), "got {err:?}");
}

#[test]
fn a_body_that_never_ends_stops_at_the_cap_without_growing() {
	// Zeros forever; the cap is one MiB, so the buffer must not pass it.
	let err = read_capped(std::io::repeat(0), MAX_METADATA_BYTES).unwrap_err();
	assert!(err.to_string().contains("maximum allowed size"), "{err}");
}

#[test]
fn metadata_hostile_to_the_parser_is_an_error_or_a_tag_never_a_panic() {
	let deep = "[".repeat(400_000);
	let deep_end = "]".repeat(400_000);
	let long_tag = format!(r#"{{"tag_name":"{}"}}"#, "9".repeat(900_000));
	let cases: Vec<(&str, String, bool)> = vec![
		("empty body", String::new(), false),
		("null", "null".into(), false),
		("array", "[]".into(), false),
		("tag is a number", r#"{"tag_name":9}"#.into(), false),
		("tag is null", r#"{"tag_name":null}"#.into(), false),
		(
			"duplicate tag",
			r#"{"tag_name":"v1.0.0","tag_name":"v9.9.9"}"#.into(),
			false,
		),
		(
			"byte order mark",
			"\u{feff}{\"tag_name\":\"v1.0.0\"}".into(),
			false,
		),
		("truncated", r#"{"tag_name":"v1.0.0"#.into(), false),
		(
			"deep nesting as the tag",
			format!(r#"{{"tag_name":{deep}{deep_end}}}"#),
			false,
		),
		(
			"deep nesting beside the tag",
			format!(r#"{{"x":{deep}{deep_end},"tag_name":"v1.0.0"}}"#),
			true,
		),
		("900 KB tag", long_tag, true),
	];
	for (what, body, parses) in cases {
		let got = parse_latest_tag(body.as_bytes());
		assert_eq!(got.is_ok(), parses, "{what}: {got:?}");
	}
}

#[test]
fn a_900_kb_tag_is_not_a_version() {
	let tag = parse_latest_tag(format!(r#"{{"tag_name":"{}"}}"#, "9".repeat(900_000)).as_bytes())
		.unwrap();
	assert!(super::super::verify::parse_version(&tag).is_err());
}

#[test]
fn a_plaintext_endpoint_is_never_connected_to() {
	// A listener on a real port, counting every connection it is offered. The
	// scheme guard has to refuse before the socket is touched: a refusal that
	// came after the connect would already have sent the request in the clear.
	let listener = TcpListener::bind("127.0.0.1:0").unwrap();
	let base = format!("http://{}", listener.local_addr().unwrap());
	let seen = Arc::new(AtomicUsize::new(0));
	let counter = seen.clone();
	std::thread::spawn(move || {
		for stream in listener.incoming() {
			counter.fetch_add(1, Ordering::SeqCst);
			drop(stream);
		}
	});
	let src = GitHubSource::with_bases_no_proxy(REPO, &base, &base);
	assert!(src.latest_version().is_err());
	assert!(src.fetch("podup-linux-x86_64").is_err());
	std::thread::sleep(std::time::Duration::from_millis(300));
	assert_eq!(
		seen.load(Ordering::SeqCst),
		0,
		"a plaintext request was sent"
	);
}
