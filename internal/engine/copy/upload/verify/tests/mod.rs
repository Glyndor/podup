use super::{
	entry_landed, format_landed_failure, sent_entries, LandedFailure, LinkCheck, SentEntry,
	SentKind,
};
use crate::engine::copy::pack::archive::pack_path;
use crate::libpod::client::PathStat;

pub(super) fn sorted(mut sent: Vec<SentEntry>) -> Vec<(String, SentKind)> {
	sent.sort_by(|a, b| a.path.cmp(&b.path));
	sent.into_iter().map(|e| (e.path, e.kind)).collect()
}

/// Drive `pack_path` into a `Vec<u8>` and return the bytes. The unified
/// packer takes a writer and a recorder; the helper builds a builder over a
/// `Vec<u8>` and discards the recorder — the tests here want the archive
/// bytes to feed `sent_entries` directly.
pub(super) fn pack_to_vec(
	src: &std::path::Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
) -> Vec<u8> {
	let mut buf = Vec::new();
	{
		let mut tar = tar::Builder::new(&mut buf);
		let mut sent = Vec::new();
		pack_path(
			src,
			follow_link,
			name_override,
			contents,
			&mut tar,
			&mut sent,
		)
		.unwrap();
	}
	buf
}

// Modes as Podman 5.7.0 reported them on 2026-09-18: 0644 file, 0755 directory.
const FILE_MODE: u64 = 420;
const DIR_MODE: u64 = 2_147_484_141;
/// 0o644 plus `os.ModeNamedPipe` (1<<25): what a FIFO at the destination looks
/// like in the stat libpod reports. The regular-file check rejects it, because
/// an empty file uploaded over an existing FIFO would otherwise be confirmed
/// by the unchanged pipe (size 0, not a directory).
const FIFO_MODE: u64 = (1 << 25) | 0o644;
/// 0o777 plus `os.ModeSymlink` (1<<27): what a symbolic link at the
/// destination looks like in the stat libpod reports.
const LINK_MODE: u64 = (1 << 27) | 0o777;

pub(super) fn stat(size: u64, mode: u64) -> PathStat {
	PathStat {
		size,
		mode,
		..PathStat::default()
	}
}

// Link-target confirmation (#1808) in its own file so this one stays under
// the line limit. A child module, so it reaches `LINK_MODE`, `FILE_MODE` and
// the imports above through `super::` without widening their visibility.
mod link;

mod archive_entries;
mod entry_landing;
mod refusal_messages;
