//! The `-L` read of a link to a directory.
//!
//! The walk that lists a tree does not descend into a link, so a link to a
//! directory is read here: its entries are listed and packed under the name
//! the link has in the archive. A link met inside it is read the same way, so a
//! chain of links is copied in full. The directories on the way down are kept,
//! and a link that leads back to one of them is refused: following it would
//! never end, and stopping at it would send a partial tree as a complete one.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{sanitize_name, ComposeError, Result};

use super::super::upload::verify::{SentEntry, SentKind};
use super::budget::LinkBudget;
use super::common::{push_entry, record_one, walk, KindDispatch};

/// Pack the link `path` to a directory under `name` when no caller holds the
/// directories above it: the one it sits in is the one being read.
pub(super) fn pack_linked_dir_entry<W, F>(
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
	name: &Path,
	path: &Path,
	map_err: F,
) -> Result<()>
where
	W: Write,
	F: Fn(&str) -> ComposeError + Copy,
{
	let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
	let mut ancestors = match parent {
		Some(dir) => vec![std::fs::canonicalize(dir).map_err(|e| map_err(&e.to_string()))?],
		None => Vec::new(),
	};
	let mut budget = LinkBudget::default();
	pack_linked_dir(tar, sent, name, path, &mut ancestors, &mut budget, map_err)
}

/// Pack the directory `path` points at under `name`, then the directory entry
/// itself. `ancestors` holds the real directories being read above this one;
/// `budget` is spent by every entry of the whole pack, so a directory reached
/// through many links cannot be copied without end.
pub(super) fn pack_linked_dir<W, F>(
	tar: &mut tar::Builder<W>,
	sent: &mut Vec<SentEntry>,
	name: &Path,
	path: &Path,
	ancestors: &mut Vec<PathBuf>,
	budget: &mut LinkBudget,
	map_err: F,
) -> Result<()>
where
	W: Write,
	F: Fn(&str) -> ComposeError + Copy,
{
	let real = std::fs::canonicalize(path).map_err(|e| map_err(&e.to_string()))?;
	if ancestors.contains(&real) {
		return Err(map_err(&format!(
			"symlink loop: {} leads back to a directory being copied",
			sanitize_name(&path.to_string_lossy())
		)));
	}
	ancestors.push(real);
	for abs in walk::walk_dir(path).map_err(|e| map_err(&e.to_string()))? {
		let rel = abs.strip_prefix(path).map_err(|_| {
			ComposeError::Build(format!(
				"cp: walk produced path outside target: {}",
				abs.display()
			))
		})?;
		let child_name = name.join(rel);
		let is_link = std::fs::symlink_metadata(&abs)
			.map_err(|e| map_err(&e.to_string()))?
			.file_type()
			.is_symlink();
		let target = std::fs::metadata(&abs).map_err(|e| map_err(&e.to_string()))?;
		budget.spend(if target.is_dir() { 0 } else { target.len() })?;
		if is_link && target.is_dir() {
			pack_linked_dir(tar, sent, &child_name, &abs, ancestors, budget, map_err)?;
		} else {
			let kind = if target.is_dir() {
				KindDispatch::Dir
			} else {
				KindDispatch::FileOrLink
			};
			record_one(tar, sent, &child_name, &abs, kind, true, map_err)?;
		}
	}
	ancestors.pop();
	budget.spend(0)?;
	tar.append_dir(name, path)
		.map_err(|e| map_err(&e.to_string()))?;
	push_entry(sent, name, SentKind::Dir)
}

#[cfg(all(test, unix))]
#[path = "linked_dir_tests.rs"]
mod tests;
