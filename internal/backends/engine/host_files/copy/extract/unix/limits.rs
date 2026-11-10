//! What one extraction is allowed to spend, however many entries the archive holds.

/// The most entries one archive may hold. The archive's byte cap bounds this
/// only indirectly, at one 512-byte header per entry; the extractor keeps an
/// identity for each file and directory it creates, so the count is bounded on
/// its own. A tree of a million entries is far above what `cp` is meant to
/// bring out of a container.
const MAX_ENTRIES: u64 = 1 << 20;

/// The most logical bytes all the sparse entries of one archive may declare
/// together. Each entry is bounded by itself; without this a stream of entries
/// that each stay under that bound costs unbounded time and apparent disk size.
pub(super) const MAX_SPARSE_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// The room left for an extraction, spent as entries are met.
pub(super) struct Limits {
	pub(super) entries: u64,
	pub(super) sparse_bytes: u64,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			entries: MAX_ENTRIES,
			sparse_bytes: MAX_SPARSE_BYTES,
		}
	}
}

impl Limits {
	/// Account for one more entry; false once the archive holds too many.
	pub(super) fn take_entry(&mut self) -> bool {
		match self.entries.checked_sub(1) {
			Some(left) => {
				self.entries = left;
				true
			}
			None => false,
		}
	}
}
