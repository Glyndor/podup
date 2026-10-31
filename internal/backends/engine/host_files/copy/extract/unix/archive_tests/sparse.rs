#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;

use super::super::{extract, Directory};

#[cfg(unix)]
#[test]
fn sparse_entries_preserve_logical_size_contents_and_holes() {
	use std::io::{Read, Seek, SeekFrom};
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let size = 4 * 1024 * 1024;
	let mut header = tar::Header::new_gnu();
	header.set_path("sparse").unwrap();
	header.set_entry_type(tar::EntryType::GNUSparse);
	header.set_size(4);
	header.set_mode(0o644);
	let gnu = header.as_gnu_mut().unwrap();
	gnu.set_real_size(size);
	gnu.sparse[0].set_offset(size - 4);
	gnu.sparse[0].set_length(4);
	header.set_cksum();
	let mut bytes = header.as_bytes().to_vec();
	bytes.extend_from_slice(b"data");
	bytes.resize(512 + 512 + 1024, 0);
	extract(&bytes[..], &root).unwrap();
	let mut output = std::fs::File::open(tmp.path().join("sparse")).unwrap();
	let metadata = output.metadata().unwrap();
	assert_eq!(metadata.len(), size);
	// Holes survive only on file systems that keep them; the macOS default
	// (APFS) allocates the whole file, so the size and content checks are the
	// part that holds everywhere.
	#[cfg(target_os = "linux")]
	assert!(
		metadata.blocks() * 512 < size / 2,
		"sparse data was materialized"
	);
	output.seek(SeekFrom::Start(size - 4)).unwrap();
	let mut tail = Vec::new();
	output.read_to_end(&mut tail).unwrap();
	assert_eq!(tail, b"data");
}

/// An entry may declare any logical size in a few bytes of header, so the size
/// is bounded before anything is read.
#[cfg(unix)]
#[test]
fn a_sparse_entry_declaring_an_absurd_size_is_refused_before_it_is_read() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let size: u64 = 1 << 50;
	let mut header = tar::Header::new_gnu();
	header.set_path("huge").unwrap();
	header.set_entry_type(tar::EntryType::GNUSparse);
	header.set_size(4);
	header.set_mode(0o644);
	let gnu = header.as_gnu_mut().unwrap();
	gnu.set_real_size(size);
	gnu.sparse[0].set_offset(size - 4);
	gnu.sparse[0].set_length(4);
	header.set_cksum();
	let mut bytes = header.as_bytes().to_vec();
	bytes.extend_from_slice(b"data");
	bytes.resize(512 + 512 + 1024, 0);
	let started = std::time::Instant::now();
	let err = extract(&bytes[..], &root).expect_err("an absurd size must be refused");
	assert!(
		started.elapsed().as_secs() < 5,
		"it must not read the zeros"
	);
	assert!(err.to_string().contains("limit"), "{err}");
}
