//! What one `-L` pack is allowed to spend, however the links in the tree fan out.
//!
//! A link to a directory is read in full wherever it is met, and the loop guard
//! only refuses a link that leads back to a directory above it. Two links per
//! level to the next level are not a loop, yet the last directory is then read
//! once per path that reaches it: 2^N times for N levels. The budget is one
//! count for the whole pack, shared by every directory read on the way, so the
//! total is bounded and not just each branch.

use crate::error::{ComposeError, Result};

/// The most entries one `-L` pack may write, the order the extractor allows an
/// archive that comes out of a container.
const MAX_ENTRIES: u64 = 1 << 20;

/// The most logical bytes of file content one `-L` pack may write.
const MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// The room left for a pack, spent as entries are met.
pub(super) struct LinkBudget {
	entries: u64,
	bytes: u64,
}

impl Default for LinkBudget {
	fn default() -> Self {
		Self::new(MAX_ENTRIES, MAX_BYTES)
	}
}

impl LinkBudget {
	pub(super) fn new(entries: u64, bytes: u64) -> Self {
		Self { entries, bytes }
	}

	/// Account for one more entry holding `bytes` of content; an error once
	/// either limit is passed.
	pub(super) fn spend(&mut self, bytes: u64) -> Result<()> {
		let entries = self.entries.checked_sub(1);
		let left = self.bytes.checked_sub(bytes);
		match (entries, left) {
			(Some(entries), Some(left)) => {
				self.entries = entries;
				self.bytes = left;
				Ok(())
			}
			(None, _) => Err(refused("entries")),
			(_, None) => Err(refused("bytes of content")),
		}
	}
}

fn refused(what: &str) -> ComposeError {
	ComposeError::Copy(format!(
		"cp: following links would copy too many {what}: the same directory is \
		 read once for every link that reaches it. Copy without -L, or copy a \
		 smaller subtree"
	))
}

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;
