//! Swap a source descendant while cp is negotiating the upload destination.
#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;

use fake_engine::{FakeEngine, Reply};
use host_files::{entries, Running, MARKER, SAFE};

static SWAP: Mutex<Option<PathBuf>> = Mutex::new(None);

fn destination(line: &str, _: &str) -> Option<Reply> {
	if line.starts_with("HEAD ") {
		if let Some(root) = SWAP.lock().unwrap().take() {
			// HEAD precedes packing: replace an existing descendant after cp starts.
			fs::rename(root.join("src/changing"), root.join("parked")).unwrap();
			std::os::unix::fs::symlink(root.join("outside"), root.join("src/changing")).unwrap();
		}
	}
	host_files::accepts(line, "")
}

#[test]
fn changing_directory_never_uploads_outside_bytes() {
	// A real stat header lets cp reach its archive PUT rather than stop at HEAD.
	let fake = FakeEngine::start_with_headers(destination,
		"X-Docker-Container-Path-Stat: eyJuYW1lIjogImRzdCIsICJzaXplIjogMCwgIm1vZGUiOiAyMTQ3NDg0MTQxLCAiaXNEaXIiOiB0cnVlfQ==\r\n");
	fs::write(
		fake.dir().join("compose.yaml"),
		"services:\n  web:\n    image: alpine\n",
	)
	.unwrap();
	fs::create_dir_all(fake.dir().join("src/changing")).unwrap();
	fs::create_dir(fake.dir().join("outside")).unwrap();
	fs::write(fake.dir().join("src/keep.txt"), SAFE).unwrap();
	fs::write(fake.dir().join("src/changing/original"), "original").unwrap();
	fs::write(fake.dir().join("outside/marker"), MARKER).unwrap();
	*SWAP.lock().unwrap() = Some(fake.dir().to_path_buf());
	let mut child = Running::start(&fake, &["cp", "src", "web:/dst/"]);
	let status = child.wait();
	assert!(status.success(), "{}", child.text());
	assert!(SWAP.lock().unwrap().is_none(), "the swap did not run");
	assert!(fake
		.dir()
		.join("src/changing")
		.symlink_metadata()
		.unwrap()
		.is_symlink());
	let requests = fake.requests();
	let uploads: Vec<_> = requests.iter().filter(|r| r.starts_with(b"PUT ")).collect();
	assert_eq!(uploads.len(), 1, "{}", fake.sent());
	let sent = entries(uploads[0]);
	assert!(sent
		.iter()
		.any(|(name, bytes)| name.ends_with("keep.txt") && bytes == SAFE.as_bytes()));
	assert!(
		sent.iter().any(|(name, _)| name.ends_with("changing")),
		"link was not packed"
	);
	assert!(
		!sent
			.iter()
			.any(|(_, bytes)| bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes())),
		"outside marker uploaded"
	);
	assert!(
		!sent.iter().any(|(name, _)| name.ends_with("/marker")),
		"outside file uploaded"
	);
	assert_eq!(
		fs::read(fake.dir().join("outside/marker")).unwrap(),
		MARKER.as_bytes()
	);
}
