//! Multi-file accumulation and a large malformed final dotenv file.
#![cfg(unix)]
use std::fs;
#[path = "../support/file_process.rs"]
mod file_process;
use file_process::{command, run, CAP, MARKER};

#[test]
fn multiple_env_files_preserve_pairs_and_reject_a_large_truncated_tail() {
	let dir = tempfile::tempdir().unwrap();
	let root = dir.path();
	let mut paths = Vec::new();
	// Each file is legal; their disjoint values exceed one file's byte cap.
	for i in 0..5 {
		let path = format!("part{i}.env");
		fs::write(
			root.join(&path),
			format!("KEY{i}={}\nORDER={i}\n", "v".repeat(CAP / 4)),
		)
		.unwrap();
		paths.push(path);
	}
	fs::write(
		root.join("compose.yaml"),
		format!(
			"services:\n  web:\n    image: alpine\n    env_file: [{}]\n",
			paths.join(", ")
		),
	)
	.unwrap();
	let control = run(command(root, &["config", "--format", "json"]));
	assert!(!control.timed_out && control.success, "{}", control.text);
	let model: serde_json::Value = serde_json::from_str(&control.text).unwrap();
	let env = &model["services"]["web"]["environment"];
	for i in 0..5 {
		assert_eq!(env[format!("KEY{i}")].as_str().unwrap().len(), CAP / 4);
	}
	assert_eq!(env["ORDER"], "4");
	fs::write(
		root.join("tail.env"),
		format!(
			"BAD\x1b[31mKEY=\"{MARKER}\n{}",
			"payload\n".repeat(CAP / 16)
		),
	)
	.unwrap();
	paths.push("tail.env".into());
	fs::write(
		root.join("compose.yaml"),
		format!(
			"services:\n  web:\n    image: alpine\n    env_file: [{}]\n",
			paths.join(", ")
		),
	)
	.unwrap();
	let bad = run(command(root, &["config", "--format", "json"]));
	assert!(
		!bad.timed_out && !bad.success,
		"truncated file was accepted"
	);
	assert!(
		bad.text.contains("unterminated quoted value"),
		"{}",
		bad.text
	);
	assert!(
		!bad.text.contains(MARKER) && !bad.text.contains('\x1b'),
		"unsafe error: {:?}",
		bad.text
	);
	assert!(bad.text.len() < 4096, "payload dumped in diagnostic");
	// This is a workload ceiling, not a claim that the product has an aggregate cap.
	for outcome in [&control, &bad] {
		assert!(
			outcome.rss_kib < 512 * 1024,
			"sampled peak RSS: {} KiB",
			outcome.rss_kib
		);
	}
	eprintln!(
		"sampled peak RSS: control={} KiB, strict={} KiB",
		control.rss_kib, bad.rss_kib
	);
}
