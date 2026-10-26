//! Shared pieces for the hostile-archive tests: a sandbox with a protected
//! "outside" tree, raw tar entry builders that bypass the safe setters, and a
//! snapshot used to prove the outside tree was not touched.

use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use tar::EntryType;

/// A destination directory and a sibling "outside" tree holding material the
/// extraction must never read, write, move or re-mode.
pub(super) struct Arena {
	_tmp: tempfile::TempDir,
	pub(super) outside: PathBuf,
	pub(super) dest: PathBuf,
	before: Vec<String>,
}

impl Arena {
	pub(super) fn new() -> Self {
		let tmp = tempfile::tempdir().expect("tempdir");
		let outside = tmp.path().join("outside");
		let dest = tmp.path().join("dest");
		std::fs::create_dir_all(outside.join("dir")).expect("mkdir outside");
		std::fs::create_dir(&dest).expect("mkdir dest");
		std::fs::write(outside.join("secret"), b"private").expect("write secret");
		std::fs::write(outside.join("dir/inner"), b"inner").expect("write inner");
		set_mode(&outside.join("secret"), 0o600);
		set_mode(&outside.join("dir"), 0o700);
		let before = snapshot(&outside);
		Self {
			_tmp: tmp,
			outside,
			dest,
			before,
		}
	}

	/// The outside tree as a string, for use as a link target.
	pub(super) fn outside_str(&self) -> String {
		let s = self.outside.to_str().expect("utf-8").to_string();
		assert!(s.len() < 80, "temp path too long for a tar link name: {s}");
		s
	}

	/// Fail when anything under the outside tree differs from the start:
	/// names, file types, modes, link targets or contents.
	pub(super) fn assert_outside_untouched(&self, what: &str) {
		let after = snapshot(&self.outside);
		assert_eq!(
			self.before, after,
			"{what}: the tree outside the destination changed"
		);
	}
}

pub(super) fn set_mode(path: &Path, mode: u32) {
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
}

fn snapshot(root: &Path) -> Vec<String> {
	let mut out = Vec::new();
	walk(root, root, &mut out);
	out.sort();
	out
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
	let mut entries: Vec<_> = std::fs::read_dir(dir)
		.expect("read_dir")
		.map(|e| e.expect("entry").path())
		.collect();
	entries.sort();
	for path in entries {
		let meta = std::fs::symlink_metadata(&path).expect("lstat");
		let rel = path.strip_prefix(root).expect("prefix").display();
		let detail = if meta.file_type().is_symlink() {
			format!(
				"-> {}",
				std::fs::read_link(&path).expect("readlink").display()
			)
		} else if meta.is_file() {
			format!("{:?}", std::fs::read(&path).expect("read"))
		} else {
			String::new()
		};
		out.push(format!(
			"{rel} type={:o} mode={:o} nlink={} {detail}",
			meta.mode() & 0o170_000,
			meta.mode() & 0o7777,
			meta.nlink()
		));
		if meta.is_dir() {
			walk(root, &path, out);
		}
	}
}

/// One 512-byte header, written field by field so `..`, absolute names and
/// NUL bytes survive (the safe setters refuse them), followed by the body
/// padded to a block.
pub(super) fn raw(name: &[u8], kind: EntryType, link: &[u8], mode: u32, data: &[u8]) -> Vec<u8> {
	assert!(name.len() <= 100 && link.len() <= 100);
	let mut header = tar::Header::new_gnu();
	header.set_entry_type(kind);
	header.set_mode(mode);
	header.set_size(data.len() as u64);
	{
		let gnu = header.as_gnu_mut().expect("gnu header");
		gnu.name[..name.len()].copy_from_slice(name);
		gnu.linkname[..link.len()].copy_from_slice(link);
	}
	header.set_cksum();
	let mut out = header.as_bytes().to_vec();
	out.extend_from_slice(data);
	out.resize(out.len() + (512 - data.len() % 512) % 512, 0);
	out
}

pub(super) fn file(name: &str, data: &[u8], mode: u32) -> Vec<u8> {
	raw(name.as_bytes(), EntryType::Regular, b"", mode, data)
}

pub(super) fn dir(name: &str, mode: u32) -> Vec<u8> {
	raw(name.as_bytes(), EntryType::Directory, b"", mode, b"")
}

pub(super) fn symlink(name: &str, target: &str) -> Vec<u8> {
	raw(
		name.as_bytes(),
		EntryType::Symlink,
		target.as_bytes(),
		0o777,
		b"",
	)
}

pub(super) fn hardlink(name: &str, target: &str) -> Vec<u8> {
	raw(
		name.as_bytes(),
		EntryType::Link,
		target.as_bytes(),
		0o666,
		b"",
	)
}

/// Concatenate entries and close the archive with the two zero blocks.
pub(super) fn archive(parts: &[Vec<u8>]) -> Vec<u8> {
	let mut out: Vec<u8> = parts.concat();
	out.resize(out.len() + 1024, 0);
	out
}

/// A PAX extended header (`x`) carrying raw `key=value` records, each with a
/// correct length prefix.
pub(super) fn pax(records: &[(&str, &[u8])]) -> Vec<u8> {
	let mut body = Vec::new();
	for (key, value) in records {
		let payload = [key.as_bytes(), b"=", value, b"\n"].concat();
		// The length counts its own digits and the space after them.
		let mut len = payload.len() + 2;
		while len.to_string().len() + 1 + payload.len() != len {
			len = len.to_string().len() + 1 + payload.len();
		}
		body.extend_from_slice(format!("{len} ").as_bytes());
		body.extend_from_slice(&payload);
	}
	raw(b"PaxHeader/x", EntryType::XHeader, b"", 0o644, &body)
}
