use super::*;

/// The request only asks for the size when the column was asked for.
///
/// Tested at this level because the string is built inside an async method that
/// needs a live socket: a mutation hard-coding `size=true` survived every test
/// that went in the front door. Asking unconditionally would make every `ps`
/// pay for a filesystem walk per container.
#[test]
fn the_request_asks_for_the_size_only_when_the_column_was_asked_for() {
	assert!(
		containers_path("demo", false, true).contains("size=true"),
		"{}",
		containers_path("demo", false, true)
	);
	assert!(
		containers_path("demo", false, false).contains("size=false"),
		"{}",
		containers_path("demo", false, false)
	);
	// The other parameters still travel, so the extraction did not drop any.
	let path = containers_path("demo", true, false);
	assert!(path.contains("all=true"), "{path}");
	assert!(path.contains("podup.project%3Ddemo"), "{path}");
}

/// The exact strings `podman ps -s` printed for these containers on
/// 2026-08-03. The column exists to be read against that output, so a
/// divergence here is a bug rather than a matter of taste.
#[test]
fn the_size_cell_matches_what_podman_prints() {
	let cases = [
		// (rw, rootFs, what podman rendered)
		(143_362_u64, 224_997_461_u64, "143kB (virtual 225MB)"),
		(11_671, 134_251_404, "11.7kB (virtual 134MB)"),
		(577_461_099, 2_082_961_972, "577MB (virtual 2.08GB)"),
		(2_084_486, 364_639_122, "2.08MB (virtual 365MB)"),
	];
	for (rw, root_fs, expected) in cases {
		let c = ContainerListEntry {
			image_id: String::new(),
			size: Some(crate::libpod::types::container::ContainerSize { rw, root_fs }),
			..entry("", "running")
		};
		assert_eq!(table_size(&c), expected, "rw={rw} rootFs={root_fs}");
	}
}

/// `virtual` is the image's own size, **not** the sum of the two. On a
/// container with a small writable layer the two are indistinguishable at three
/// significant digits, so this uses the three real containers whose readings
/// actually differ; otherwise the test passes under either reading and pins
/// nothing.
#[test]
fn virtual_is_the_image_size_and_not_the_total() {
	let rw = 577_461_099;
	let root_fs = 2_082_961_972;
	let c = ContainerListEntry {
		image_id: String::new(),
		size: Some(crate::libpod::types::container::ContainerSize { rw, root_fs }),
		..entry("", "running")
	};
	let cell = table_size(&c);
	assert!(
		cell.contains("2.08GB"),
		"{cell:?} should report the image size"
	);
	assert!(
		!cell.contains("2.66GB"),
		"{cell:?} reported the sum, which is not what podman calls virtual"
	);
}

/// A container whose size was never requested renders an empty cell rather than
/// a zero. libpod omits the field unless the query asked, so a zero here would
/// claim podup asked and the answer was nothing.
#[test]
fn a_size_that_was_not_requested_leaves_the_cell_empty() {
	assert_eq!(table_size(&entry("", "running")), "");
}

/// A genuinely empty writable layer still renders, so the blank above is keyed
/// on "not asked" and not on "small".
#[test]
fn a_zero_byte_writable_layer_still_renders() {
	let c = ContainerListEntry {
		image_id: String::new(),
		size: Some(crate::libpod::types::container::ContainerSize {
			rw: 0,
			root_fs: 1_000_000,
		}),
		..entry("", "running")
	};
	assert_eq!(table_size(&c), "0B (virtual 1.00MB)");
}
