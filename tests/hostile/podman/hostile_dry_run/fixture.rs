//! All mutable paths, including the bind's parent, live under one snapshot.

use std::fs;
use std::path::PathBuf;

use tempfile::TempDir;

use super::peer::{Peer, Request};
use super::run::{podup, Outcome};
use super::snapshot::snapshot;

pub const COMPOSE: &str = "services:\n  web:\n    image: alpine:3.20\n    build: .\n    volumes:\n      - data:/data\n    develop:\n      watch:\n        - action: sync\n          path: ./watched\n          target: /watched\nvolumes:\n  data: {}\n";
pub const MISSING_BIND: &str = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - type: bind\n        source: ../x\n        target: /missing\n        bind:\n          create_host_path: true\n";

pub struct Fixture {
	pub root: TempDir,
	pub project: PathBuf,
	pub home: PathBuf,
}

impl Fixture {
	pub fn new(compose: &str) -> Self {
		let root = tempfile::tempdir().expect("workspace");
		let project = root.path().join("project");
		let home = root.path().join("home");
		for dir in [&project, &home, &home.join("existing/empty")] {
			fs::create_dir_all(dir).unwrap();
		}
		fs::write(project.join("compose.yaml"), compose).unwrap();
		fs::write(project.join("Containerfile"), "FROM alpine:3.20\n").unwrap();
		fs::write(project.join("watched"), b"do not sync me\n").unwrap();
		fs::write(home.join("existing/keep"), b"\0\xffkeep HOME intact\n").unwrap();
		std::os::unix::fs::symlink("existing/keep", home.join("keep-link")).unwrap();
		Self {
			root,
			project,
			home,
		}
	}

	pub fn exercise(&self, args: &[&str]) -> (Outcome, Vec<Request>) {
		// The socket lives elsewhere so the peer's own activity is not mistaken
		// for a podup write. Join/drain it before checking its complete log.
		let peer = Peer::start();
		let before = snapshot(self.root.path());
		let outcome = podup(self, &peer.sock, args);
		let requests = peer.finish();
		let after = snapshot(self.root.path());
		assert_eq!(
			after,
			before,
			"{args:?}: project/HOME/parent changed\n{}\nrequests: {requests:#?}",
			outcome.text()
		);
		assert!(
			requests.iter().all(Request::read_only),
			"{args:?}: mutating or hijack request: {requests:#?}\n{}",
			outcome.text()
		);
		assert!(
			outcome.code.is_some(),
			"{args:?}: killed/timed out\n{}",
			outcome.text()
		);
		assert!(
			!outcome.text().contains("panicked at"),
			"{args:?}: {}",
			outcome.text()
		);
		(outcome, requests)
	}
}
