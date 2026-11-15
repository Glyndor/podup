//! Record at the request head, before replying or waiting for a POST body.

use std::io::{self, Read};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use tempfile::TempDir;

#[derive(Debug)]
pub struct Request {
	pub method: String,
	pub target: String,
	upgrade: bool,
}

impl Request {
	pub fn read_only(&self) -> bool {
		matches!(self.method.as_str(), "GET" | "HEAD") && !self.upgrade
	}
}

pub struct Peer {
	pub sock: PathBuf,
	stop: Arc<AtomicBool>,
	seen: Arc<Mutex<Vec<Request>>>,
	worker: Option<JoinHandle<()>>,
	_dir: TempDir,
}

impl Peer {
	pub fn start() -> Self {
		let dir = tempfile::tempdir().expect("socket directory");
		let sock = dir.path().join("podman.sock");
		let listener = UnixListener::bind(&sock).expect("bind fake Podman");
		listener.set_nonblocking(true).unwrap();
		let stop = Arc::new(AtomicBool::new(false));
		let seen = Arc::new(Mutex::new(Vec::new()));
		let (flag, log) = (stop.clone(), seen.clone());
		let worker = std::thread::spawn(move || loop {
			match listener.accept() {
				Ok((mut stream, _)) => {
					stream
						.set_read_timeout(Some(Duration::from_secs(2)))
						.unwrap();
					stream
						.set_write_timeout(Some(Duration::from_secs(2)))
						.unwrap();
					let request = read_head(&mut stream);
					// Log even an incomplete/malformed request as forbidden.
					let mut entries = log.lock().unwrap();
					entries.push(request);
					super::responses::answer(&mut stream, entries.last().unwrap());
				}
				Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
					// Drain queued connections after child exit before ending the log.
					if flag.load(Ordering::SeqCst) {
						break;
					}
					std::thread::sleep(Duration::from_millis(5));
				}
				Err(e) => panic!("accept fake Podman: {e}"),
			}
		});
		Self {
			sock,
			stop,
			seen,
			worker: Some(worker),
			_dir: dir,
		}
	}

	pub fn finish(mut self) -> Vec<Request> {
		self.stop.store(true, Ordering::SeqCst);
		self.worker
			.take()
			.unwrap()
			.join()
			.expect("join recording peer");
		let requests = std::mem::take(&mut *self.seen.lock().unwrap());
		requests
	}
}

impl Drop for Peer {
	fn drop(&mut self) {
		self.stop.store(true, Ordering::SeqCst);
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

fn read_head(stream: &mut UnixStream) -> Request {
	let mut head = Vec::new();
	let mut byte = [0u8; 1];
	while !head.ends_with(b"\r\n\r\n") && head.len() < 64 * 1024 {
		if !matches!(stream.read(&mut byte), Ok(1)) {
			break;
		}
		head.push(byte[0]);
	}
	let text = String::from_utf8_lossy(&head);
	let mut words = text.lines().next().unwrap_or("").split_whitespace();
	let method = words.next().unwrap_or("<invalid>").to_string();
	let target = words.next().unwrap_or("<invalid>").to_string();
	let upgrade = !head.ends_with(b"\r\n\r\n")
		|| text.lines().skip(1).any(|line| {
			let Some((name, value)) = line.split_once(':') else {
				return false;
			};
			name.trim().eq_ignore_ascii_case("upgrade")
				|| (name.trim().eq_ignore_ascii_case("connection")
					&& value
						.split(',')
						.any(|v| v.trim().eq_ignore_ascii_case("upgrade")))
		}) || target.contains("/attach")
		|| (target.contains("/exec/") && target.split('?').next().unwrap().ends_with("/start"));
	Request {
		method,
		target,
		upgrade,
	}
}
