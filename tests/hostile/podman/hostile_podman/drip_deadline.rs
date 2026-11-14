//! A busy socket cannot renew the total buffered-response deadline.

use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::run::{assert_refused, podup};
use super::server::{self, ping_ok};

#[test]
fn one_byte_reads_do_not_extend_the_response_deadline() {
	let sent = Arc::new(AtomicUsize::new(0));
	let progress = sent.clone();
	let fake = server::start(move |stream, request| {
		if request.target.contains("_ping") {
			ping_ok(stream);
			return;
		}
		stream
			.set_write_timeout(Some(Duration::from_secs(1)))
			.unwrap();
		stream
			.write_all(
				b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
				  Content-Length: 1000000\r\nConnection: close\r\n\r\n[",
			)
			.unwrap();
		// JSON whitespace keeps the unfinished array valid. Every write is one
		// byte, far below the size cap and well inside any 120 s idle timeout.
		// Outlive the watchdog so peer EOF cannot masquerade as a client timeout.
		let start = Instant::now();
		while start.elapsed() < Duration::from_secs(160) {
			if stream.write_all(b" ").is_err() {
				return;
			}
			progress.fetch_add(1, Ordering::SeqCst);
			std::thread::sleep(Duration::from_millis(100));
		}
	});
	let home = tempfile::tempdir().unwrap();
	let outcome = podup(&fake.sock, &home, &["ps"], Duration::from_secs(150));
	assert_refused("one-byte drip", &outcome);
	assert!(
		outcome.took >= Duration::from_secs(110) && outcome.took < Duration::from_secs(140),
		"expected the 120 s response deadline, took {:?}",
		outcome.took
	);
	assert!(
		outcome.text.contains("timed out"),
		"expected a client timeout error: {}",
		outcome.text
	);
	assert!(
		sent.load(Ordering::SeqCst) >= 100,
		"the peer did not sustain the one-byte drip"
	);
}
