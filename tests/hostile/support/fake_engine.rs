//! A Unix socket that answers like a Podman engine, for tests that need podup
//! to get past its first request without a real engine.
//!
//! It answers `_ping` with a supported API version, a missing secret or inspected object with a
//! 404, a `GET` with an empty list and anything else with an object holding an
//! id. A [`Handler`] overrides any request it recognises, and every request
//! (head and body) is recorded so a test can check what was sent.
//!
//! Every integration test file is its own crate, so a test declares this with
//! `#[path = "../support/fake_engine.rs"] mod fake_engine;`.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use tempfile::TempDir;

/// What to answer: a status code and a JSON body.
pub struct Reply {
	pub status: u16,
	pub body: String,
}

/// Looks at the request line and the whole request. `None` keeps the default.
pub type Handler = fn(&str, &str) -> Option<Reply>;

pub struct FakeEngine {
	dir: TempDir,
	seen: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl FakeEngine {
	pub fn start(handler: Handler) -> Self {
		Self::start_with_headers(handler, "")
	}

	/// Add response headers for archive-stat protocol controls.
	pub fn start_with_headers(handler: Handler, headers: &'static str) -> Self {
		let dir = tempfile::tempdir().expect("temp dir");
		// The project lock stages in this directory and refuses one that others
		// can enter, whatever the umask of the machine running the tests.
		std::fs::set_permissions(
			dir.path(),
			std::os::unix::fs::PermissionsExt::from_mode(0o700),
		)
		.expect("chmod");
		let listener = UnixListener::bind(dir.path().join("podman.sock")).expect("bind socket");
		let seen = Arc::new(Mutex::new(Vec::new()));
		let log = seen.clone();
		std::thread::spawn(move || {
			for stream in listener.incoming() {
				let Ok(stream) = stream else { break };
				serve(stream, handler, &log, headers);
			}
		});
		Self { dir, seen }
	}

	/// The directory podup runs in; put the compose file and its siblings here.
	pub fn dir(&self) -> &std::path::Path {
		self.dir.path()
	}

	/// Every request received so far, head and body, one entry each.
	pub fn sent(&self) -> String {
		self.requests()
			.iter()
			.map(|r| String::from_utf8_lossy(r).into_owned())
			.collect::<Vec<_>>()
			.join("\n")
	}

	/// Raw requests, preserving gzip and tar bytes without UTF-8 conversion.
	pub fn requests(&self) -> Vec<Vec<u8>> {
		self.seen.lock().unwrap().clone()
	}

	/// Run podup against this engine and return everything it printed.
	pub fn run(&self, log: Option<&str>, env: &[(&str, &str)], args: &[&str]) -> String {
		let socket = self.dir.path().join("podman.sock");
		let mut cmd = Command::new(env!("CARGO_BIN_EXE_podup"));
		cmd.current_dir(self.dir.path())
			.arg("--socket")
			.arg(socket)
			.args(args)
			.envs(env.iter().copied())
			.env("NO_COLOR", "1")
			.env_remove("RUST_LOG")
			.stdin(Stdio::null());
		if let Some(level) = log {
			cmd.env("RUST_LOG", level);
		}
		let out = cmd.output().expect("run podup");
		format!(
			"{}\n{}",
			String::from_utf8_lossy(&out.stdout),
			String::from_utf8_lossy(&out.stderr)
		)
	}
}

/// Read one request, answer it and close.
fn serve(mut stream: UnixStream, handler: Handler, log: &Mutex<Vec<Vec<u8>>>, headers: &str) {
	let mut data = Vec::new();
	let mut chunk = [0u8; 4096];
	let (head_end, length) = loop {
		let Ok(n) = stream.read(&mut chunk) else {
			return;
		};
		if n == 0 {
			return;
		}
		data.extend_from_slice(&chunk[..n]);
		let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") else {
			continue;
		};
		let head = String::from_utf8_lossy(&data[..at]).to_lowercase();
		if head.contains("transfer-encoding: chunked") {
			while !data.ends_with(b"0\r\n\r\n") {
				match stream.read(&mut chunk) {
					Ok(0) | Err(_) => break,
					Ok(n) => data.extend_from_slice(&chunk[..n]),
				}
			}
			break (at + 4, 0);
		}
		let length = head
			.lines()
			.find_map(|l| l.strip_prefix("content-length:"))
			.and_then(|v| v.trim().parse::<usize>().ok())
			.unwrap_or(0);
		break (at + 4, length);
	};
	while data.len() < head_end + length {
		match stream.read(&mut chunk) {
			Ok(0) | Err(_) => break,
			Ok(n) => data.extend_from_slice(&chunk[..n]),
		}
	}
	let request = String::from_utf8_lossy(&data).into_owned();
	let line = request.lines().next().unwrap_or_default().to_string();
	log.lock().unwrap().push(data);

	let ping = line.contains("/libpod/_ping");
	let reply = handler(&line, &request).unwrap_or_else(|| default_reply(&line));
	let version = if ping {
		"Libpod-API-Version: 5.4.0\r\n"
	} else {
		""
	};
	let wire = format!(
		"HTTP/1.1 {} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
		 connection: close\r\n{version}{headers}\r\n{}",
		reply.status,
		reply.body.len(),
		reply.body
	);
	let _ = stream.write_all(wire.as_bytes());
	let _ = stream.shutdown(std::net::Shutdown::Both);
}

fn default_reply(line: &str) -> Reply {
	let (status, body) = if line.contains("/libpod/_ping") {
		(200, "OK")
	} else if line.starts_with("GET") && line.contains("/secrets/") {
		(
			404,
			"{\"cause\":\"no such secret\",\"message\":\"no such secret\",\"response\":404}",
		)
	} else if line.starts_with("GET") && is_object_inspect(line) {
		// A real engine answers an inspect of an object that does not exist with
		// a 404; an empty list there would read as an object without a label.
		(
			404,
			"{\"cause\":\"no such object\",\"message\":\"no such object\",\"response\":404}",
		)
	} else if line.starts_with("GET") {
		(200, "[]")
	} else {
		(200, "{\"Id\":\"0123456789abcdef\"}")
	};
	Reply {
		status,
		body: body.to_string(),
	}
}

/// Whether the request line is the inspect of one container, pod, volume or
/// network (`/<kind>/<name>/json`), not a list (`/<kind>/json`).
fn is_object_inspect(line: &str) -> bool {
	let target = line.split_whitespace().nth(1).unwrap_or("");
	let path = target.split('?').next().unwrap_or("");
	["/containers/", "/pods/", "/volumes/", "/networks/"]
		.iter()
		.any(|kind| {
			path.contains(kind)
				&& path.ends_with("/json")
				&& !path.ends_with(&format!("{}json", kind))
		})
}
