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

/// Every `alpine@sha256:<64 hex>` or `alpine:<tag>@sha256:<64 hex>` in `text`,
/// as the `sha256:<64 hex>` part.
fn digests_in(text: &str) -> Vec<String> {
	let mut found = Vec::new();
	let mut rest = text;
	while let Some(at) = rest.find("alpine") {
		let mut tail = &rest[at + "alpine".len()..];
		if let Some(after_colon) = tail.strip_prefix(':') {
			if let Some((tag, _)) = after_colon.split_once('@') {
				if !tag.is_empty()
					&& tag
						.chars()
						.all(|c| c.is_ascii_alphanumeric() || ".-_".contains(c))
				{
					tail = &tail[1 + tag.len()..];
				}
			}
		}
		if let Some(tail) = tail.strip_prefix('@') {
			let digest: String = tail.chars().take("sha256:".len() + 64).collect();
			if digest.starts_with("sha256:")
				&& digest.len() == "sha256:".len() + 64
				&& digest[7..].chars().all(|c| c.is_ascii_hexdigit())
			{
				found.push(digest);
			}
		}
		rest = &rest[at + "alpine".len()..];
	}
	found
}

/// Dependabot's docker ecosystem reads a `FROM` line as `image:tag@digest`.
/// A digest with no tag gives it no release stream to follow, so the pin goes
/// stale without a pull request ever being opened for it.
fn from_line_has_a_tag(dockerfile: &str) -> bool {
	dockerfile.lines().any(|line| {
		line.strip_prefix("FROM ").is_some_and(|image| {
			image
				.split_once('@')
				.is_some_and(|(name, _)| name.rsplit('/').next().is_some_and(|n| n.contains(':')))
		})
	})
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
	assert!(
		from_line_has_a_tag(&dockerfile),
		"{SOURCE} pins a digest with no tag (`alpine:<tag>@sha256:...`), so Dependabot cannot bump it"
	);
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

#[test]
fn a_tag_beside_the_digest_is_read_and_a_bare_digest_is_refused() {
	let d = format!("sha256:{}", "c".repeat(64));
	assert_eq!(
		digests_in(&format!("FROM x/alpine:3.24@{d}\n")),
		std::slice::from_ref(&d)
	);
	assert_eq!(
		digests_in(&format!("image: x/alpine@{d}\n")),
		std::slice::from_ref(&d)
	);
	assert!(digests_in("FROM x/alpine:3.24\n").is_empty());
	assert!(from_line_has_a_tag(&format!("FROM x/alpine:3.24@{d}\n")));
	assert!(!from_line_has_a_tag(&format!("FROM x/alpine@{d}\n")));
	// A port in the registry host is not a tag.
	assert!(!from_line_has_a_tag(&format!(
		"FROM host:5000/alpine@{d}\n"
	)));
}
