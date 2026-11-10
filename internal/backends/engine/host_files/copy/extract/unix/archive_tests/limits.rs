//! What one extraction may spend however many entries the archive holds.

use super::super::super::hostile_support::{archive, file};
use super::super::limits::Limits;
use super::super::{extract_limited, Directory};

#[test]
fn an_archive_with_more_entries_than_the_limit_is_refused() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let parts: Vec<Vec<u8>> = (0..5)
		.map(|i| file(&format!("f{i}"), b"x", 0o644))
		.collect();
	let limits = Limits { entries: 4 };
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
	let limits = Limits { entries: 4 };
	if let Err(e) = extract_limited(&archive(&parts)[..], &root, limits) {
		panic!("four entries fit a limit of four: {e}");
	}
	assert!(tmp.path().join("f3").exists());
}
