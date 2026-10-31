//! Every relative link in the README, CONTRIBUTING and `docs/` must land on a
//! file that exists and, when it names a `#anchor`, on a heading that produces
//! it. `docs/security-model.md` pointed at `commands.md#audit` while the `audit`
//! heading lives in `commands-reference.md`, and nothing noticed because the
//! flags test reads headings, never links.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

fn root() -> PathBuf {
	PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn markdown_files() -> Vec<PathBuf> {
	let mut files = vec![root().join("README.md"), root().join("CONTRIBUTING.md")];
	for entry in fs::read_dir(root().join("docs")).unwrap() {
		let path = entry.unwrap().path();
		if path.extension().is_some_and(|e| e == "md") {
			files.push(path);
		}
	}
	files.retain(|p| p.exists());
	files
}

/// GitHub's heading slug: lowercase, backticks and punctuation dropped, spaces
/// to hyphens.
fn slug(heading: &str) -> String {
	heading
		.trim()
		.to_lowercase()
		.chars()
		.filter_map(|c| match c {
			' ' | '-' => Some('-'),
			c if c.is_alphanumeric() || c == '_' => Some(c),
			_ => None,
		})
		.collect()
}

/// The anchors a file's headings produce, outside fenced code blocks.
fn anchors(text: &str) -> BTreeSet<String> {
	let mut fenced = false;
	let mut out = BTreeSet::new();
	for line in text.lines() {
		if line.trim_start().starts_with("```") {
			fenced = !fenced;
			continue;
		}
		if !fenced && line.starts_with('#') {
			out.insert(slug(line.trim_start_matches('#')));
		}
	}
	out
}

/// `](target)` links outside fenced code blocks and inline code spans.
fn links(text: &str) -> Vec<String> {
	let mut fenced = false;
	let mut out = Vec::new();
	for line in text.lines() {
		if line.trim_start().starts_with("```") {
			fenced = !fenced;
			continue;
		}
		if fenced {
			continue;
		}
		let plain: String = line
			.split('`')
			.enumerate()
			.filter(|(i, _)| i % 2 == 0)
			.map(|(_, s)| s)
			.collect::<Vec<_>>()
			.join(" ");
		let mut rest = plain.as_str();
		while let Some(at) = rest.find("](") {
			let after = &rest[at + 2..];
			let Some(end) = after.find(')') else { break };
			out.push(after[..end].to_string());
			rest = &after[end..];
		}
	}
	out
}

/// Every broken link in `files`, as `file: target (reason)`.
fn broken_links(files: &[PathBuf]) -> Vec<String> {
	let mut broken = Vec::new();
	for file in files {
		let text = fs::read_to_string(file).unwrap();
		let dir = file.parent().unwrap();
		for target in links(&text) {
			if target.contains("://") || target.starts_with("mailto:") {
				continue;
			}
			let (path, anchor) = match target.split_once('#') {
				Some((p, a)) => (p, Some(a)),
				None => (target.as_str(), None),
			};
			let dest: PathBuf = if path.is_empty() {
				file.clone()
			} else {
				dir.join(path)
			};
			let name = file.strip_prefix(root()).unwrap_or(file).display();
			if !dest.exists() {
				broken.push(format!("{name}: {target} (no such file)"));
				continue;
			}
			if let Some(anchor) = anchor.filter(|_| dest.extension().is_some_and(|e| e == "md")) {
				let found = anchors(&fs::read_to_string(&dest).unwrap());
				if !found.contains(anchor) {
					broken.push(format!("{name}: {target} (no heading makes #{anchor})"));
				}
			}
		}
	}
	broken
}

#[test]
fn every_relative_link_in_the_docs_lands() {
	let broken = broken_links(&markdown_files());
	assert!(broken.is_empty(), "{}", broken.join("\n"));
}

/// The check has to see what it claims to see: a wrong file, a wrong anchor and
/// a right link in a scratch tree.
#[test]
fn the_check_catches_a_wrong_file_and_a_wrong_anchor() {
	let dir = std::env::temp_dir().join(format!("podup-docs-links-{}", std::process::id()));
	fs::create_dir_all(&dir).unwrap();
	fs::write(dir.join("a.md"), "# One\n\n## `audit` flags\n").unwrap();
	fs::write(
		dir.join("b.md"),
		"[ok](a.md#audit-flags) [file](missing.md) [anchor](a.md#audit) [self](#top)\n\n# Top\n\n```\n[skipped](nope.md)\n```\n",
	)
	.unwrap();
	let _guard = Cleanup(dir.clone());
	let found = broken_links(&[dir.join("b.md")]);
	assert_eq!(found.len(), 2, "{found:?}");
	assert!(found[0].contains("missing.md"), "{found:?}");
	assert!(found[1].contains("#audit)"), "{found:?}");
}

struct Cleanup(PathBuf);

impl Drop for Cleanup {
	fn drop(&mut self) {
		let _ = fs::remove_dir_all(&self.0);
	}
}
