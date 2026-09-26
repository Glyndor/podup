//! The running binary applies the Podman floor.
//!
//! `podman_floor.rs` pins that the floor is the same number everywhere it is
//! written. This file pins that the binary uses it: every command that talks
//! to Podman sends one `GET /libpod/_ping` first and stops, with the message
//! written for the case, when the engine reports a libpod API major below
//! the floor. Until 5.10.2 nothing called that check, so on Podman 4.9 a
//! command failed on its first real request with whatever that request
//! returned (#1924).
//!
//! The engine here is a Unix socket served from a thread: it answers `_ping`
//! with the `Libpod-API-Version` header the test chooses, and everything else
//! with an empty JSON list, so `ls` against it has nothing to show and exits
//! 0 once the check lets it through.

use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::sync::{Arc, Mutex};

#[cfg(unix)]
struct FakePodman {
	dir: tempfile::TempDir,
	/// The request line of every request answered, in order.
	requests: Arc<Mutex<Vec<String>>>,
}

#[cfg(unix)]
impl FakePodman {
	/// A socket whose `_ping` reports `api_version`, or no version header at
	/// all for `None`.
	fn start(api_version: Option<&'static str>) -> Self {
		let dir = tempfile::tempdir().unwrap();
		let listener =
			std::os::unix::net::UnixListener::bind(dir.path().join("podman.sock")).unwrap();
		let requests = Arc::new(Mutex::new(Vec::new()));
		let seen = requests.clone();
		std::thread::spawn(move || {
			for stream in listener.incoming() {
				let Ok(stream) = stream else {
					break;
				};
				serve_one(stream, api_version, &seen);
			}
		});
		Self { dir, requests }
	}

	fn socket(&self) -> String {
		self.dir.path().join("podman.sock").display().to_string()
	}

	fn requests(&self) -> Vec<String> {
		self.requests.lock().unwrap().clone()
	}

	/// Run the built podup against this socket, from an empty directory so no
	/// compose file of the checkout is read.
	fn podup(&self, args: &[&str]) -> std::process::Output {
		std::process::Command::new(env!("CARGO_BIN_EXE_podup"))
			.arg("--socket")
			.arg(self.socket())
			.args(args)
			.env("NO_COLOR", "1")
			.current_dir(self.dir.path())
			.output()
			.unwrap()
	}
}

/// Answer one HTTP/1.1 request and close. Every request podup sends here has
/// no body, so the header block is the whole request.
#[cfg(unix)]
fn serve_one(
	mut stream: std::os::unix::net::UnixStream,
	api_version: Option<&str>,
	seen: &Mutex<Vec<String>>,
) {
	use std::io::{Read, Write};

	let mut head = Vec::new();
	let mut chunk = [0u8; 1024];
	while !head.windows(4).any(|w| w == b"\r\n\r\n") {
		match stream.read(&mut chunk) {
			Ok(0) | Err(_) => return,
			Ok(n) => head.extend_from_slice(&chunk[..n]),
		}
	}
	let head = String::from_utf8_lossy(&head);
	let request_line = head.lines().next().unwrap_or_default().to_string();
	seen.lock().unwrap().push(request_line.clone());
	let target = request_line.split_whitespace().nth(1).unwrap_or_default();

	let (content_type, version_header, body) = if target.ends_with("/libpod/_ping") {
		let header = api_version
			.map(|v| format!("Libpod-API-Version: {v}\r\n"))
			.unwrap_or_default();
		("text/plain", header, "OK")
	} else {
		("application/json", String::new(), "[]")
	};
	let response = format!(
		"HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\
		 connection: close\r\n{version_header}\r\n{body}",
		body.len()
	);
	let _ = stream.write_all(response.as_bytes());
	let _ = stream.shutdown(std::net::Shutdown::Both);
}

#[cfg(unix)]
#[test]
fn an_engine_below_the_floor_is_refused_before_any_other_request() {
	let fake = FakePodman::start(Some("4.9.3"));
	let out = fake.podup(&["ls"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert_eq!(out.status.code(), Some(1), "stderr: {stderr}");
	assert!(
		stderr
			.contains("podup requires Podman >= 5.0; this server reports libpod API version 4.9.3"),
		"the message written for an unsupported Podman must be the one shown: {stderr}"
	);
	let requests = fake.requests();
	assert_eq!(
		requests.len(),
		1,
		"the check must be the only request sent to an engine below the floor: {requests:?}"
	);
	assert!(
		requests[0].starts_with("GET ") && requests[0].contains("/libpod/_ping"),
		"the one request must be the ping: {requests:?}"
	);
}

/// No `Libpod-API-Version` header at all fails closed: a socket that does not
/// say what it speaks is not assumed to speak a supported libpod.
#[cfg(unix)]
#[test]
fn an_engine_that_reports_no_version_is_refused() {
	let fake = FakePodman::start(None);
	let out = fake.podup(&["ls"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert_eq!(out.status.code(), Some(1), "stderr: {stderr}");
	assert!(
		stderr.contains("this server reports libpod API version an unknown version"),
		"{stderr}"
	);
	assert_eq!(fake.requests().len(), 1, "{:?}", fake.requests());
}

#[cfg(unix)]
#[test]
fn an_engine_at_the_floor_is_checked_first_and_then_used() {
	let fake = FakePodman::start(Some("5.0.0"));
	let out = fake.podup(&["ls"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(
		out.status.success(),
		"status {:?}, stderr: {stderr}",
		out.status
	);
	let requests = fake.requests();
	assert!(
		requests
			.first()
			.is_some_and(|r| r.starts_with("GET ") && r.contains("/libpod/_ping")),
		"the ping must come before anything else: {requests:?}"
	);
	assert!(
		requests
			.iter()
			.skip(1)
			.any(|r| r.contains("/libpod/containers/json")),
		"after the check the command must go on to its own request: {requests:?}"
	);
}

/// Every client the command line builds goes through the checked constructor.
///
/// The check is one call in one function, so a new command path that connects
/// through `podman::connect` or `podman::connect_from_env` (both public,
/// because the integration tests use them) would skip it and nothing else
/// would say so. Comments are not counted: a doc comment naming the
/// unchecked constructor is not a call.
#[test]
fn every_client_the_command_line_builds_is_checked() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"));
	for rel in ["internal/main.rs", "internal/autostart_cmd.rs"] {
		let src = fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
		let code = src
			.lines()
			.filter(|l| !l.trim_start().starts_with("//"))
			.collect::<Vec<_>>()
			.join("\n");
		let checked = code.matches("podman::connect_checked(").count();
		let unchecked = ["podman::connect(", "podman::connect_from_env"]
			.iter()
			.map(|needle| code.matches(needle).count())
			.sum::<usize>();
		assert!(
			checked >= 1,
			"{rel} builds no client through connect_checked"
		);
		assert_eq!(
			unchecked, 0,
			"{rel} builds a client without the version check; use connect_checked"
		);
	}
}
