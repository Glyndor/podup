//! Build contexts written to make the packer read, hang or leak: ignore files
//! and Dockerfiles that are links out of the tree, special files, loops and
//! synthesized entries with traversal names.

use std::sync::mpsc;
use std::time::Duration;

use flate2::read::GzDecoder;
use std::io::Read;

use super::*;

/// `build_context_tar` on a thread with a budget: a walk or a read that never
/// ends is the failure under test, and a hung test would hide it.
fn tar_within_budget(
	dir: &Path,
	extra: Vec<(String, Vec<u8>)>,
) -> crate::error::Result<Vec<(String, tar::EntryType, Vec<u8>)>> {
	let dir = dir.to_path_buf();
	let (tx, rx) = mpsc::channel();
	std::thread::spawn(move || {
		let result = build_context_tar(&dir, "Dockerfile", &extra).map(|gz| {
			let mut raw = Vec::new();
			GzDecoder::new(gz.as_slice())
				.read_to_end(&mut raw)
				.expect("gunzip");
			let mut archive = tar::Archive::new(raw.as_slice());
			let mut out = Vec::new();
			for entry in archive.entries().expect("entries") {
				let mut entry = entry.expect("entry");
				let path = entry.path().expect("path").to_string_lossy().into_owned();
				let kind = entry.header().entry_type();
				let mut body = Vec::new();
				entry.read_to_end(&mut body).expect("body");
				out.push((path, kind, body));
			}
			out
		});
		let _ = tx.send(result);
	});
	rx.recv_timeout(Duration::from_secs(30))
		.expect("the context tar did not finish within 30 seconds")
}

fn names(entries: &[(String, tar::EntryType, Vec<u8>)]) -> Vec<&str> {
	entries.iter().map(|(n, _, _)| n.as_str()).collect()
}

fn leaked(entries: &[(String, tar::EntryType, Vec<u8>)], marker: &[u8]) -> bool {
	entries
		.iter()
		.any(|(_, _, body)| body.windows(marker.len()).any(|w| w == marker))
}

#[cfg(unix)]
#[test]
fn an_ignore_file_that_is_a_link_out_of_the_tree_is_not_read() {
	// If the link were followed, the outside file's `*` would drop the whole
	// context, and its content would steer what ships.
	let outside = tempdir().unwrap();
	fs::write(outside.path().join("rules"), b"*\n").unwrap();
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
	fs::write(dir.path().join("app.txt"), b"app").unwrap();
	std::os::unix::fs::symlink(
		outside.path().join("rules"),
		dir.path().join(".dockerignore"),
	)
	.unwrap();
	let entries = tar_within_budget(dir.path(), Vec::new()).unwrap();
	assert!(
		names(&entries).contains(&"app.txt"),
		"{:?}",
		names(&entries)
	);
}

#[cfg(unix)]
#[test]
fn links_out_of_the_tree_never_carry_the_bytes_they_point_at() {
	let outside = tempdir().unwrap();
	fs::create_dir(outside.path().join("keys")).unwrap();
	fs::write(outside.path().join("keys/id"), b"OUTSIDE-MARKER").unwrap();
	fs::write(outside.path().join("df"), b"OUTSIDE-MARKER").unwrap();
	let dir = tempdir().unwrap();
	std::os::unix::fs::symlink(outside.path().join("df"), dir.path().join("Dockerfile")).unwrap();
	std::os::unix::fs::symlink(outside.path().join("keys"), dir.path().join("keys")).unwrap();
	std::os::unix::fs::symlink("/etc/passwd", dir.path().join("passwd")).unwrap();
	let entries = tar_within_budget(dir.path(), Vec::new()).unwrap();
	assert!(!leaked(&entries, b"OUTSIDE-MARKER"));
	assert!(!leaked(&entries, b"root:"));
	assert!(!names(&entries).iter().any(|n| n.starts_with("keys/")));
	for (_, kind, body) in &entries {
		if *kind == tar::EntryType::Symlink {
			assert!(body.is_empty());
		}
	}
}

#[cfg(unix)]
#[test]
fn a_link_to_the_context_itself_does_not_loop() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
	std::os::unix::fs::symlink(".", dir.path().join("self")).unwrap();
	std::os::unix::fs::symlink("..", dir.path().join("parent")).unwrap();
	let entries = tar_within_budget(dir.path(), Vec::new()).unwrap();
	assert_eq!(entries.len(), 3, "{:?}", names(&entries));
}

#[cfg(unix)]
#[test]
fn a_fifo_in_the_context_ends_in_an_error_and_is_never_opened() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
	let status = std::process::Command::new("mkfifo")
		.arg(dir.path().join("pipe"))
		.status()
		.expect("mkfifo");
	assert!(status.success());
	assert!(tar_within_budget(dir.path(), Vec::new()).is_err());
}

#[cfg(unix)]
#[test]
fn a_socket_in_the_context_ends_in_an_error() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
	let _listener = std::os::unix::net::UnixListener::bind(dir.path().join("sock")).unwrap();
	assert!(tar_within_budget(dir.path(), Vec::new()).is_err());
}

#[test]
fn a_synthesized_entry_cannot_name_a_path_outside_the_archive() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("Dockerfile"), b"FROM alpine\n").unwrap();
	for name in ["../escape", "a/../../escape", "/etc/escape"] {
		let result = tar_within_budget(dir.path(), vec![(name.to_string(), b"x".to_vec())]);
		if let Ok(entries) = result {
			let landed: Vec<&str> = names(&entries)
				.into_iter()
				.filter(|n| n.contains("escape"))
				.collect();
			assert!(
				landed
					.iter()
					.all(|n| !n.starts_with('/') && !n.contains("..")),
				"{name} became {landed:?}"
			);
		}
	}
}
