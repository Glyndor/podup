//! The unit tests of the sync filter, one file per subject.
//!
//! The suite was a single `sync_filter_tests.rs` over the 300-line soft
//! limit, so it became a folder: this module keeps the shared helpers and the
//! index, and each leaf holds the scenarios of one subject.

use super::{build_sync_tar, has_permitted_entry};
use crate::engine::watch::actions::entry_is_unsyncable;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs;
use std::io::Read;
use std::path::Path;
use tempfile::tempdir;

/// Drive the unified `build_sync_tar` with a filter closure, and return
/// the gzipped tar bytes together with the `SentEntry` list the packer
/// recorded. Used by tests that want to assert the recorded list
/// alongside the archive contents so a sabotage that drops an entry from
/// the tar but leaves it in the recorder (or vice versa) is caught.
fn sync_to_gz_with_sent(
	src: &Path,
	entry_name: &Path,
	skip: &dyn Fn(&Path) -> bool,
) -> std::io::Result<(Vec<u8>, Vec<crate::engine::copy::verify::SentEntry>)> {
	// On macOS the temporary directory sits under a link, and the sync refuses a
	// link in any component of the path it is given.
	let src = &src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
	let buf = Vec::new();
	let encoder = GzEncoder::new(buf, Compression::default());
	let mut tar = tar::Builder::new(encoder);
	let mut sent = Vec::new();
	build_sync_tar(src, entry_name, &mut tar, &mut sent, skip)
		.map_err(|e| std::io::Error::other(e.to_string()))?;
	let encoder = tar.into_inner().map_err(std::io::Error::other)?;
	let bytes = encoder.finish().map_err(std::io::Error::other)?;
	Ok((bytes, sent))
}

/// Decode a gzipped tar and return `(path, kind)` for every entry that has
/// one (`Kind` is "file", "dir", or "link"). Ordering is the archive order,
/// which is the order the packer appended the entries.
fn tar_entries(gz: &[u8]) -> Vec<(String, &'static str)> {
	let mut decoder = GzDecoder::new(gz);
	let mut raw = Vec::new();
	decoder.read_to_end(&mut raw).unwrap();
	let mut archive = tar::Archive::new(&raw[..]);
	let mut out = Vec::new();
	for entry in archive.entries().unwrap() {
		let entry = entry.unwrap();
		let path = entry
			.path()
			.unwrap()
			.to_string_lossy()
			.replace('\\', "/")
			.to_string();
		let kind = if entry.header().entry_type().is_dir() {
			"dir"
		} else if entry.header().entry_type().is_symlink() {
			"link"
		} else if entry.header().entry_type().is_file() {
			"file"
		} else {
			continue;
		};
		out.push((path, kind));
	}
	out
}

mod filter;
mod permitted;
#[cfg(unix)]
mod pipe;
mod swaps;
mod unsyncable;
