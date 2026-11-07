//! No tracked file points at a document that is not in this repository.
//!
//! Comments and messages that cite the private standards tree send a reader to
//! a path they cannot open, and name a tree the public repository does not
//! have. The needles are built from parts so this file does not match itself.

use std::process::Command;

#[test]
fn no_tracked_file_cites_a_private_document_tree() {
	let needles = [
		format!("{}{}", "ai-", "context"),
		format!("{}{}", "standards", "/"),
	];
	// A source tarball (the Debian build) has no git history to list.
	let Some(listing) = Command::new("git")
		.args(["ls-files", "-z"])
		.current_dir(env!("CARGO_MANIFEST_DIR"))
		.output()
		.ok()
		.filter(|out| out.status.success())
	else {
		eprintln!("git cannot list the files here, so there is nothing to check");
		return;
	};

	let mut found = Vec::new();
	for name in String::from_utf8_lossy(&listing.stdout).split('\0') {
		if name.is_empty() || name == "Cargo.lock" {
			continue;
		}
		let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
		let Ok(text) = std::fs::read_to_string(&path) else {
			continue;
		};
		for (at, line) in text.lines().enumerate() {
			if needles.iter().any(|needle| line.contains(needle.as_str())) {
				found.push(format!("{name}:{}: {}", at + 1, line.trim()));
			}
		}
	}
	assert!(
		found.is_empty(),
		"references to a document tree that is not in this repository:\n  {}",
		found.join("\n  ")
	);
}
