//! Links that fan out are read under one budget, not once per path.

use std::path::{Path, PathBuf};

use super::super::budget::LinkBudget;
use super::pack_linked_dir;

/// `level0` holds two links to `level1`, which holds two links to `level2`, and
/// so on; the last level holds one file. No link leads back, so the loop guard
/// accepts it, and the file is reached by 2^levels different paths.
fn fan_out(root: &Path, levels: usize) -> PathBuf {
	for i in 0..=levels {
		std::fs::create_dir(root.join(format!("level{i}"))).expect("mkdir");
	}
	for i in 0..levels {
		for link in ["a", "b"] {
			std::os::unix::fs::symlink(
				root.join(format!("level{}", i + 1)),
				root.join(format!("level{i}")).join(link),
			)
			.expect("symlink");
		}
	}
	std::fs::write(root.join(format!("level{levels}/data")), b"x").expect("write");
	root.join("level0")
}

fn pack(top: &Path, mut budget: LinkBudget) -> (crate::error::Result<()>, usize) {
	let mut sent = Vec::new();
	let result = {
		let mut tar = tar::Builder::new(Vec::new());
		pack_linked_dir(
			&mut tar,
			&mut sent,
			Path::new("top"),
			top,
			&mut Vec::new(),
			&mut budget,
			|msg| crate::error::ComposeError::Copy(msg.to_string()),
		)
	};
	(result, sent.len())
}

/// Without a limit the copy doubles at every level: this is the growth the
/// budget exists to stop.
#[test]
fn without_a_limit_the_copies_double_at_every_level() {
	let mut counts = Vec::new();
	for levels in [4, 6, 8] {
		let dir = tempfile::tempdir().expect("tempdir");
		let top = fan_out(dir.path(), levels);
		let (result, entries) = pack(&top, LinkBudget::new(u64::MAX, u64::MAX));
		result.expect("pack");
		counts.push(entries);
	}
	assert!(
		counts[1] > 3 * counts[0] && counts[2] > 3 * counts[1],
		"{counts:?}"
	);
}

#[test]
fn a_fan_out_that_passes_the_budget_is_refused_with_the_way_out() {
	let dir = tempfile::tempdir().expect("tempdir");
	let top = fan_out(dir.path(), 14);
	let (result, entries) = pack(&top, LinkBudget::new(5_000, u64::MAX));
	let err = result.expect_err("the copy must stop").to_string();
	assert!(err.contains("too many entries"), "{err}");
	assert!(err.contains("Copy without -L"), "{err}");
	assert!(entries <= 5_000, "{entries} entries were recorded");
}

#[test]
fn a_tree_with_a_few_links_fits_the_default_budget() {
	let dir = tempfile::tempdir().expect("tempdir");
	let top = fan_out(dir.path(), 3);
	let (result, entries) = pack(&top, LinkBudget::default());
	result.expect("pack");
	assert!(entries > 8, "{entries}");
}
