//! Tests for `pack_path`, the unified `cp` packer.
//!
//! These tests drive the production function directly: the unified packer
//! takes a `tar::Builder<W: Write>` and a recorder, so the tests build a
//! builder over a `Vec<u8>` and read the archive back through the same API
//! the streaming path uses. The parity tests in `transport_tests.rs` compare the
//! recorder against `sent_entries` of these bytes; the tests here check the
//! archive shape (`pack_path_entry_names`) and the error categories.

use std::io::Read;
use std::path::Path;

/// Drive `pack_path` into a `Vec<u8>` and return the bytes, mirroring what
/// the streaming path produces on the wire. The unified packer takes a
/// `tar::Builder<W: Write>` and a recorder; the helper builds a builder over
/// a `Vec<u8>` so the test can read the archive back and assert on its
/// shape. The recorder is dropped — the parity check on it lives in
/// `transport_tests.rs`.
pub(super) fn pack_to_vec(
	src: &Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
) -> Vec<u8> {
	let mut buf = Vec::new();
	{
		let mut tar = tar::Builder::new(&mut buf);
		let mut sent = Vec::new();
		super::pack_path(
			src,
			follow_link,
			name_override,
			contents,
			&mut tar,
			&mut sent,
		)
		.expect("pack_path");
	}
	buf
}

/// Drive `pack_path` and return both the bytes and the recorded entry list.
/// Drives the parity test in `transport_tests.rs` (`recorded == sent_entries` of
/// the same bytes), so every test that uses it pins that the recorder and
/// the archive agree.
pub(super) fn pack_with_recorder(
	src: &Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
) -> (Vec<u8>, Vec<crate::engine::copy::verify::SentEntry>) {
	let mut buf = Vec::new();
	let mut sent = Vec::new();
	{
		let mut tar = tar::Builder::new(&mut buf);
		super::pack_path(
			src,
			follow_link,
			name_override,
			contents,
			&mut tar,
			&mut sent,
		)
		.expect("pack_path");
	}
	(buf, sent)
}

/// Read the entry names of the tar built by `pack_path`, which is plain tar
/// since the bytes are handed to a local Unix socket. Sorted because the
/// walker goes in directory order, which is the filesystem's business.
pub(super) fn pack_path_entry_names(bytes: &[u8]) -> Vec<String> {
	let mut archive = tar::Archive::new(bytes);
	let mut names: Vec<String> = Vec::new();
	for entry in archive.entries().expect("entries") {
		let entry = entry.expect("entry");
		let path = entry.path().expect("path");
		// Skip the top-level wrapper a directory archive adds; we want the
		// inner shape, which is what `cp` actually puts in the container.
		for component in path.components() {
			if let std::path::Component::Normal(p) = component {
				names.push(p.to_string_lossy().into_owned());
			}
		}
	}
	names.sort();
	names
}

mod archive_shape;
#[cfg(unix)]
mod hostile;
#[cfg(unix)]
mod pinned;
mod symlinks;
