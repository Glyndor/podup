use super::*;

// --- entry_is_unsyncable ------------------------------------------------

/// An entry whose component is a non-UTF-8 byte (`<0xff>`) is
/// unsyncable: the lossy form of `container_rel` substitutes the
/// replacement character, so this name would collide with the
/// other bytes that map to the same character. The write-back
/// filter inside `sync_to_container` drops the entry, and the gate
/// in `maybe_sync` short-circuits the whole sync for a non-UTF-8
/// top-level path so the lossy mapping never runs.
#[cfg(unix)]
#[test]
fn entry_is_unsyncable_true_for_non_utf8_component() {
	use std::ffi::OsStr;
	use std::os::unix::ffi::OsStrExt;
	let p = Path::new(OsStr::from_bytes(b"\xff")).join("keep.txt");
	assert!(
		entry_is_unsyncable(&p),
		"a path whose component is non-UTF-8 must be reported unsyncable"
	);
}

/// A plain ASCII path is syncable: the filter inside
/// `sync_to_container` runs the write-back check, not the UTF-8
/// check, and the gate in `maybe_sync` lets the dispatch reach it.
#[test]
fn entry_is_unsyncable_false_for_utf8_path() {
	let p = Path::new("ok").join("keep.txt");
	assert!(
		!entry_is_unsyncable(&p),
		"a UTF-8 path must not be reported unsyncable"
	);
}
