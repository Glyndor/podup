//! Bounded child execution and lossless archive inspection for hostile inputs.
#![allow(dead_code)]

use std::fs::{self, File};
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use super::fake_engine::{FakeEngine, Reply};

pub const MARKER: &str = "OUTSIDE-SECRET-H1-7d932af";
pub const SAFE: &str = "IN-TREE-CONTROL-H1-914a";
pub const BUDGET: Duration = Duration::from_secs(10);

pub struct Running {
	child: Child,
	out: tempfile::NamedTempFile,
	err: tempfile::NamedTempFile,
}

impl Running {
	pub fn start(fake: &FakeEngine, args: &[&str]) -> Self {
		let out = tempfile::NamedTempFile::new().unwrap();
		let err = tempfile::NamedTempFile::new().unwrap();
		let child = Command::new(env!("CARGO_BIN_EXE_podup"))
			.current_dir(fake.dir())
			.args(["--socket", fake.dir().join("podman.sock").to_str().unwrap()])
			.args(["-f", "compose.yaml", "-p", "hostile"])
			.args(args)
			.env("HOME", fake.dir())
			.env("XDG_CONFIG_HOME", fake.dir().join("config"))
			.env("XDG_DATA_HOME", fake.dir().join("data"))
			.env("XDG_RUNTIME_DIR", fake.dir())
			.env("NO_COLOR", "1")
			.env_remove("RUST_LOG")
			.env_remove("COMPOSE_PROFILES")
			.stdin(Stdio::null())
			.stdout(Stdio::from(File::create(out.path()).unwrap()))
			.stderr(Stdio::from(File::create(err.path()).unwrap()))
			.spawn()
			.unwrap();
		Self { child, out, err }
	}

	pub fn text(&self) -> String {
		fs::read_to_string(self.out.path()).unwrap() + &fs::read_to_string(self.err.path()).unwrap()
	}

	pub fn wait(&mut self) -> ExitStatus {
		let start = Instant::now();
		loop {
			if let Some(status) = self.child.try_wait().unwrap() {
				return status;
			}
			assert!(start.elapsed() < BUDGET, "child blocked: {}", self.text());
			std::thread::sleep(Duration::from_millis(10));
		}
	}

	pub fn until(&mut self, needle: &str) {
		let start = Instant::now();
		while !self.text().contains(needle) {
			assert!(
				self.child.try_wait().unwrap().is_none(),
				"child exited: {}",
				self.text()
			);
			assert!(
				start.elapsed() < BUDGET,
				"waiting for {needle}: {}",
				self.text()
			);
			std::thread::sleep(Duration::from_millis(10));
		}
	}
}

impl Drop for Running {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

pub fn accepts(line: &str, _: &str) -> Option<Reply> {
	if line.contains("/images/") && line.ends_with("/json HTTP/1.1") {
		Some(Reply {
			status: 200,
			body: "{\"Id\":\"sha256:123\",\"Config\":{}}".into(),
		})
	} else if line.contains("/exec/") && line.contains("/start") {
		Some(Reply {
			status: 200,
			body: String::new(),
		})
	} else if line.contains("/exec/") && line.contains("/json") {
		Some(Reply {
			status: 200,
			body: "{\"Running\":false,\"ExitCode\":0}".into(),
		})
	} else {
		None
	}
}

/// Decode HTTP framing before looking inside a tar or gzip stream.
pub fn body(request: &[u8]) -> Vec<u8> {
	let at = request.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
	let head = String::from_utf8_lossy(&request[..at]).to_ascii_lowercase();
	if !head.contains("transfer-encoding: chunked") {
		return request[at..].to_vec();
	}
	let mut rest = &request[at..];
	let mut decoded = Vec::new();
	while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
		let size = usize::from_str_radix(
			std::str::from_utf8(&rest[..end])
				.unwrap()
				.split(';')
				.next()
				.unwrap(),
			16,
		)
		.unwrap();
		if size == 0 {
			break;
		}
		rest = &rest[end + 2..];
		let available = size.min(rest.len());
		decoded.extend_from_slice(&rest[..available]);
		if rest.len() < size + 2 {
			break;
		}
		rest = &rest[size + 2..];
	}
	decoded
}

pub fn entries(request: &[u8]) -> Vec<(String, Vec<u8>)> {
	let body = body(request);
	let reader: Box<dyn Read> = if body.starts_with(&[0x1f, 0x8b]) {
		Box::new(flate2::read::GzDecoder::new(body.as_slice()))
	} else {
		Box::new(body.as_slice())
	};
	let mut tar = tar::Archive::new(reader);
	tar.entries()
		.unwrap()
		.map(|entry| {
			let mut entry = entry.unwrap();
			let name = entry.path().unwrap().to_string_lossy().into_owned();
			let mut bytes = Vec::new();
			entry.read_to_end(&mut bytes).unwrap();
			(name, bytes)
		})
		.collect()
}

/// Inspect bytes already sent even when the producer aborted an archive.
pub fn partial_archive_bytes(request: &[u8]) -> Vec<u8> {
	let bytes = body(request);
	if !bytes.starts_with(&[0x1f, 0x8b]) {
		return bytes;
	}
	let mut decoded = Vec::new();
	// A truncated gzip stream can still contain bytes that escaped to the daemon.
	let _ = flate2::read::GzDecoder::new(bytes.as_slice()).read_to_end(&mut decoded);
	decoded
}
