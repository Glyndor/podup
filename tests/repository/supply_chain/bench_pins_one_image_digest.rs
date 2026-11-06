//! Every benchmark scenario pins the same alpine digest as the build Dockerfile.
//!
//! The digest was written out 57 times across 14 scenarios, and Dependabot's
//! docker entry reads only `bench/scenarios/build/Dockerfile`. A bump there
//! would have left 56 references on the old image, and the benchmark would have
//! compared tools that ran different images. The Dockerfile is the source;
//! `bench/lib/sync-image-digest.sh` copies its digest into the scenarios and
//! this test fails while any of them differs, naming the script.
//!
//! A source package may leave `bench/` out, and then there is nothing to read.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const SOURCE: &str = "bench/scenarios/build/Dockerfile";

/// Every `alpine@sha256:<64 hex>` in `text`, as the part after `alpine@`.
fn digests_in(text: &str) -> Vec<String> {
	let mut found = Vec::new();
	let mut rest = text;
	while let Some(at) = rest.find("alpine@sha256:") {
		let tail = &rest[at + "alpine@".len()..];
		let digest: String = tail.chars().take("sha256:".len() + 64).collect();
		if digest.len() == "sha256:".len() + 64
			&& digest[7..].chars().all(|c| c.is_ascii_hexdigit())
		{
			found.push(digest);
		}
		rest = &rest["alpine@".len() + at..];
	}
	found
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
	for entry in fs::read_dir(dir).unwrap().flatten() {
		let path = entry.path();
		if path.is_dir() {
			files_under(&path, out);
		} else {
			out.push(path);
		}
	}
}

/// One line per file whose digests are not exactly `source`, and a floor so a
/// scan that finds nothing cannot pass.
fn problems(source: &str, files: &BTreeMap<String, String>) -> Vec<String> {
	let mut out = Vec::new();
	let mut references = 0;
	for (name, text) in files {
		for digest in digests_in(text) {
			references += 1;
			if digest != source {
				out.push(format!(
					"{name} pins {digest}, the Dockerfile pins {source}"
				));
			}
		}
	}
	if references < 10 {
		out.push(format!(
			"only {references} references found; the scan reads the wrong files"
		));
	}
	if !out.is_empty() {
		out.push("run bench/lib/sync-image-digest.sh".to_string());
	}
	out
}

#[test]
fn every_scenario_pins_the_digest_of_the_build_dockerfile() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"));
	let source_path = root.join(SOURCE);
	if !source_path.exists() {
		return;
	}
	let dockerfile = fs::read_to_string(&source_path).unwrap();
	let sources = digests_in(&dockerfile);
	assert_eq!(
		sources.len(),
		1,
		"{SOURCE} must pin exactly one alpine digest: {sources:?}"
	);
	let mut paths = Vec::new();
	files_under(&root.join("bench/scenarios"), &mut paths);
	let files: BTreeMap<String, String> = paths
		.into_iter()
		.filter_map(|p| {
			let text = fs::read_to_string(&p).ok()?;
			Some((p.strip_prefix(root).unwrap().display().to_string(), text))
		})
		.collect();
	let found = problems(&sources[0], &files);
	assert!(found.is_empty(), "\n{}\n", found.join("\n"));
}

#[test]
fn a_different_digest_and_an_empty_scan_are_both_reported() {
	let a = format!("sha256:{}", "a".repeat(64));
	let b = format!("sha256:{}", "b".repeat(64));
	let mut files = BTreeMap::new();
	for n in 0..10 {
		files.insert(
			format!("s{n}/compose.yaml"),
			format!("image: docker.io/library/alpine@{a}\n"),
		);
	}
	assert!(problems(&a, &files).is_empty());
	files.insert(
		"x/compose.yaml".into(),
		format!("image: docker.io/library/alpine@{b}\n"),
	);
	let found = problems(&a, &files);
	assert!(found.iter().any(|l| l.contains("x/compose.yaml pins")));
	assert!(found.last().unwrap().contains("sync-image-digest.sh"));
	assert!(!problems(&a, &BTreeMap::new()).is_empty());
}
