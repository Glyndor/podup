//! Label file read failures must warn; invalid pairs must never reach create.
#![cfg(unix)]
use std::fs;
use std::process::Command;
#[path = "../support/fake_engine.rs"]
mod fake_engine;
#[path = "../support/host_files.rs"]
mod host_files;
use fake_engine::FakeEngine;
use host_files::{accepts, body, Running};

const CAP: usize = 16 * 1024 * 1024;
const MARKER: &str = "LABEL-H3-CANARY-51c2";

#[test]
fn label_file_types_bytes_and_limits_are_enforced_before_create() {
	for case in [
		"valid", "near-cap", "fifo", "device", "link", "oversize", "utf8", "nul", "escape",
	] {
		let fake = FakeEngine::start(accepts);
		let root = fake.dir();
		let path = root.join("labels\x1b[31m.txt");
		let mut label_path = path.to_string_lossy().into_owned();
		match case {
			"valid" => fs::write(&path, "com.example.control=honest\n").unwrap(),
			"near-cap" => {
				let mut content = format!("com.example.control=honest\n#{}", "c".repeat(CAP - 32));
				content.truncate(CAP - 1);
				fs::write(&path, content).unwrap();
			}
			"fifo" => assert!(Command::new("mkfifo")
				.arg(&path)
				.status()
				.unwrap()
				.success()),
			"device" => label_path = "/dev/zero".into(),
			"link" => {
				fs::write(
					root.join("canary"),
					format!("com.example.hostile={MARKER}\n"),
				)
				.unwrap();
				std::os::unix::fs::symlink(root.join("canary"), &path).unwrap();
			}
			"oversize" => {
				let mut content = format!("com.example.hostile={MARKER}\n#");
				content.extend(std::iter::repeat_n('c', CAP + 1 - content.len()));
				fs::write(&path, content).unwrap();
			}
			"utf8" => fs::write(&path, b"com.example.hostile=\xff\n").unwrap(),
			"nul" => fs::write(&path, format!("com.example.hostile={MARKER}\0bad\n")).unwrap(),
			_ => fs::write(&path, format!("com.example.hostile={MARKER}\x1b[31m\n")).unwrap(),
		}
		fs::write(
			root.join("compose.yaml"),
			format!(
				"services:\n  web:\n    image: alpine\n    label_file: {}\n",
				serde_json::to_string(&label_path).unwrap()
			),
		)
		.unwrap();
		let mut child = Running::start(&fake, &["create"]);
		let status = child.wait();
		let text = child.text();
		assert!(
			!text.contains('\x1b') && !text.contains(MARKER),
			"{case}: unsafe diagnostic {text:?}"
		);
		let requests = fake.requests();
		let creates: Vec<_> = requests
			.iter()
			.filter(|r| {
				let head = String::from_utf8_lossy(r);
				head.lines()
					.next()
					.unwrap_or("")
					.contains("/containers/create")
			})
			.collect();
		if matches!(case, "nul" | "escape") {
			assert!(
				!status.success() && creates.is_empty(),
				"{case}: invalid labels reached create: {text}"
			);
			assert!(
				text.contains("label_file") && text.contains("rejected"),
				"{case}: {text}"
			);
		} else {
			assert!(status.success(), "{case}: {text}");
			assert_eq!(creates.len(), 1, "{case}: did not reach create: {text}");
			let spec: serde_json::Value = serde_json::from_slice(&body(creates[0])).unwrap();
			if matches!(case, "valid" | "near-cap") {
				assert_eq!(
					spec["labels"]["com.example.control"], "honest",
					"{case}: {spec}"
				);
			} else {
				assert!(
					text.contains("label_file: cannot read"),
					"{case}: missing read warning: {text}"
				);
				assert!(
					spec["labels"].get("com.example.hostile").is_none(),
					"{case}: hostile pair sent"
				);
			}
		}
	}
}
