//! What one extraction may spend however many entries the archive holds.

use super::super::super::hostile_support::{archive, file};
use super::super::limits::Limits;
use super::super::{extract_limited, Directory};

fn sparse_entry(name: &str, size: u64) -> Vec<u8> {
	let mut header = tar::Header::new_gnu();
	header.set_path(name).unwrap();
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
	bytes.resize(512 + 512, 0);
	bytes
}

#[test]
fn an_archive_with_more_entries_than_the_limit_is_refused() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let parts: Vec<Vec<u8>> = (0..5)
		.map(|i| file(&format!("f{i}"), b"x", 0o644))
		.collect();
	let limits = Limits {
		entries: 4,
		..Limits::default()
	};
	let err = extract_limited(&archive(&parts)[..], &root, limits)
		.err()
		.expect("a fifth entry must be refused");
	assert!(err.to_string().contains("too many entries"), "{err}");
	assert!(!tmp.path().join("f4").exists());
}

#[test]
fn an_archive_with_exactly_the_limit_of_entries_is_extracted() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let parts: Vec<Vec<u8>> = (0..4)
		.map(|i| file(&format!("f{i}"), b"x", 0o644))
		.collect();
	let limits = Limits {
		entries: 4,
		..Limits::default()
	};
	if let Err(e) = extract_limited(&archive(&parts)[..], &root, limits) {
		panic!("four entries fit a limit of four: {e}");
	}
	assert!(tmp.path().join("f3").exists());
}

#[test]
fn sparse_entries_that_each_fit_but_together_exceed_the_total_are_refused() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let parts = [
		sparse_entry("one", 600 * 1024),
		sparse_entry("two", 600 * 1024),
	];
	let limits = Limits {
		sparse_bytes: 1024 * 1024,
		..Limits::default()
	};
	let err = extract_limited(&archive(&parts)[..], &root, limits)
		.err()
		.expect("the second entry must be refused");
	assert!(err.to_string().contains("total limit"), "{err}");
}
