use super::{build_sync_tar, legacy_project_relative_ignored, legacy_project_relative_included};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use std::fs;
use std::io::Read;
use std::path::Path;
use tempfile::tempdir;

fn pats(v: &[&str]) -> Vec<String> {
	v.iter().map(|s| s.to_string()).collect()
}

/// `src` with the directory above it resolved. Only the last component is left
/// alone, since a test may hand the sync a link on purpose.
fn resolved_parent(src: &Path) -> std::path::PathBuf {
	match (src.parent(), src.file_name()) {
		(Some(parent), Some(name)) => parent
			.canonicalize()
			.map_or_else(|_| src.to_path_buf(), |p| p.join(name)),
		_ => src.to_path_buf(),
	}
}

/// Drive the unified `build_sync_tar` into a `Vec<u8>` and return the bytes.
/// Mirrors the streaming path: the writer is a gzip encoder over the `Vec`,
/// exactly the way `build_sync_tar_stream` wraps the channel writer.
fn sync_to_gz(src: &Path, entry_name: &Path) -> std::io::Result<Vec<u8>> {
	// On macOS the temporary directory sits under a link, and the sync refuses a
	// link in any component of the path it is given.
	let src = &resolved_parent(src);
	let buf = Vec::new();
	let encoder = GzEncoder::new(buf, Compression::default());
	let mut tar = tar::Builder::new(encoder);
	let mut sent = Vec::new();
	build_sync_tar(src, entry_name, &mut tar, &mut sent, &|_| false)
		.map_err(|e| std::io::Error::other(e.to_string()))?;
	let encoder = tar.into_inner().map_err(std::io::Error::other)?;
	encoder.finish().map_err(std::io::Error::other)
}

/// Same shape as `sync_to_gz`, but with a filter closure. Used by the
/// `build_sync_tar` tests that exercise the per-entry skip.
fn sync_to_gz_with(
	src: &Path,
	entry_name: &Path,
	skip: &dyn Fn(&Path) -> bool,
) -> std::io::Result<Vec<u8>> {
	// On macOS the temporary directory sits under a link, and the sync refuses a
	// link in any component of the path it is given.
	let src = &resolved_parent(src);
	let buf = Vec::new();
	let encoder = GzEncoder::new(buf, Compression::default());
	let mut tar = tar::Builder::new(encoder);
	let mut sent = Vec::new();
	build_sync_tar(src, entry_name, &mut tar, &mut sent, skip)
		.map_err(|e| std::io::Error::other(e.to_string()))?;
	let encoder = tar.into_inner().map_err(std::io::Error::other)?;
	encoder.finish().map_err(std::io::Error::other)
}
/// Decode a gzipped tar and collect its non-directory entry paths.
fn tar_entry_paths(gz: &[u8]) -> Vec<String> {
	let mut decoder = GzDecoder::new(gz);
	let mut raw = Vec::new();
	decoder.read_to_end(&mut raw).unwrap();
	let mut archive = tar::Archive::new(&raw[..]);
	let mut names = Vec::new();
	for entry in archive.entries().unwrap() {
		let entry = entry.unwrap();
		if entry.header().entry_type().is_file() {
			names.push(entry.path().unwrap().to_string_lossy().replace('\\', "/"));
		}
	}
	names
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

/// Read the link target of a single entry of a gzipped tar. Returns `None`
/// when the entry is not a symlink.
fn tar_link_target(gz: &[u8], entry_name: &str) -> Option<String> {
	let mut decoder = GzDecoder::new(gz);
	let mut raw = Vec::new();
	decoder.read_to_end(&mut raw).unwrap();
	let mut archive = tar::Archive::new(&raw[..]);
	for entry in archive.entries().unwrap() {
		let entry = entry.unwrap();
		let path = entry
			.path()
			.unwrap()
			.to_string_lossy()
			.replace('\\', "/")
			.to_string();
		if path != entry_name {
			continue;
		}
		if !entry.header().entry_type().is_symlink() {
			return None;
		}
		return entry
			.link_name()
			.unwrap()
			.map(|p| p.to_string_lossy().replace('\\', "/").to_string());
	}
	None
}

// legacy_project_relative_ignored -----------------------------------------------------------

mod directory_entries;
mod legacy_patterns;
mod skip_filter;
mod symlinks;
mod tar_entries;
