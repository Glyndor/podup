//! `write_units` is handed names derived from compose keys. Whatever a key
//! looks like, a unit lands as a plain file directly inside the output
//! directory, and a name that cannot be one is refused before anything is
//! written.

use super::{write_units, QuadletUnit};

fn unit(filename: &str) -> QuadletUnit {
	QuadletUnit {
		filename: filename.to_string(),
		contents: "[Container]\nImage=alpine\n".to_string(),
	}
}

#[test]
fn a_name_that_is_not_one_plain_component_is_refused_and_nothing_is_written() {
	for name in [
		"../escape.container",
		"a/../../escape.container",
		"sub/escape.container",
		"/abs/escape.container",
		"..",
		".",
		"",
		"trailing/",
		"..//x",
	] {
		let root = tempfile::tempdir().expect("tempdir");
		let out = root.path().join("out");
		let err = write_units(&out, &[unit(name)]).expect_err(name);
		assert_eq!(
			err.kind(),
			std::io::ErrorKind::InvalidInput,
			"{name}: {err}"
		);
		let leftovers: Vec<_> = std::fs::read_dir(root.path())
			.expect("read_dir")
			.map(|e| e.expect("entry").path())
			.collect();
		assert!(
			leftovers.iter().all(|p| p == &out),
			"{name} wrote outside the output directory: {leftovers:?}"
		);
		assert!(
			std::fs::read_dir(&out).expect("out dir").next().is_none(),
			"{name} wrote into the output directory"
		);
	}
}

#[test]
fn a_bad_name_after_a_good_one_still_writes_nothing_outside_the_directory() {
	let root = tempfile::tempdir().expect("tempdir");
	let out = root.path().join("out");
	let err = write_units(&out, &[unit("ok.container"), unit("../bad.container")])
		.expect_err("the second name is unsafe");
	assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
	assert!(!root.path().join("bad.container").exists());
}

#[test]
fn an_output_path_that_is_a_file_is_an_error() {
	let root = tempfile::tempdir().expect("tempdir");
	let out = root.path().join("out");
	std::fs::write(&out, "not a directory").expect("write");
	assert!(write_units(&out, &[unit("a.container")]).is_err());
	assert_eq!(std::fs::read(&out).expect("read"), b"not a directory");
}
