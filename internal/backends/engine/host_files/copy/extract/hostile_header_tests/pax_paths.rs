//! Each PAX payload gets a fresh filesystem and archive so refusal cannot mask it.

use std::path::Path;

use super::super::hostile_support::{archive, file, pax, symlink, Arena};
use super::run;
use crate::error::ComposeError;

#[test]
fn relative_pax_path_is_refused() {
	let arena = Arena::new();
	let tar = archive(&[
		pax(&[("path", b"../outside/secret")]),
		file("innocent", b"pwn", 0o644),
	]);
	let result = run(&arena, &tar);
	arena.assert_outside_untouched("relative PAX path");
	assert_escape(result, "../outside/secret");
	assert!(!arena.dest.join("innocent").exists());
}

#[test]
fn absolute_pax_path_is_rooted_inside_destination() {
	let arena = Arena::new();
	let absolute = arena.outside.join("secret");
	let tar = archive(&[
		pax(&[("path", absolute.as_os_str().as_encoded_bytes())]),
		file("innocent", b"pwn", 0o644),
	]);
	let result = run(&arena, &tar);
	arena.assert_outside_untouched("absolute PAX path");
	result.expect("absolute names are safely rooted inside the destination");
	let relative = absolute
		.strip_prefix(Path::new("/"))
		.expect("absolute path");
	assert_eq!(std::fs::read(arena.dest.join(relative)).unwrap(), b"pwn");
	assert!(!arena.dest.join("innocent").exists());
}

#[test]
fn pax_linkpath_is_stored_but_never_traversed() {
	let arena = Arena::new();
	let target = arena.outside_str();
	let tar = archive(&[
		pax(&[("linkpath", target.as_bytes())]),
		symlink("lnk", "x"),
		file("lnk/secret", b"pwn", 0o644),
	]);
	let result = run(&arena, &tar);
	arena.assert_outside_untouched("PAX linkpath traversal");
	assert_eq!(
		std::fs::read_link(arena.dest.join("lnk")).unwrap(),
		Path::new(&target)
	);
	let ComposeError::Io(error) = result.expect_err("a symlink parent must be refused") else {
		panic!("expected a filesystem refusal");
	};
	assert!(
		matches!(error.raw_os_error(), Some(libc::ENOTDIR | libc::ELOOP)),
		"{error}"
	);
}

#[test]
fn valid_pax_path_override_is_consumed_for_one_entry() {
	let arena = Arena::new();
	let tar = archive(&[
		pax(&[("path", b"renamed")]),
		file("placeholder", b"payload", 0o644),
		file("after", b"next", 0o644),
	]);
	let result = run(&arena, &tar);
	arena.assert_outside_untouched("valid PAX override");
	result.expect("valid PAX path");
	assert_eq!(
		std::fs::read(arena.dest.join("renamed")).unwrap(),
		b"payload"
	);
	assert_eq!(std::fs::read(arena.dest.join("after")).unwrap(), b"next");
	let mut names: Vec<_> = std::fs::read_dir(&arena.dest)
		.unwrap()
		.map(|entry| entry.unwrap().file_name())
		.collect();
	names.sort();
	assert_eq!(names, ["after", "renamed"]);
}

#[test]
fn pax_component_of_300_bytes_is_refused() {
	let arena = Arena::new();
	let name = "a".repeat(300);
	let tar = archive(&[
		pax(&[("path", name.as_bytes())]),
		file("placeholder", b"pwn", 0o644),
	]);
	let result = run(&arena, &tar);
	arena.assert_outside_untouched("300-byte PAX component");
	let ComposeError::Io(error) =
		result.expect_err("the filesystem refuses an oversized component")
	else {
		panic!("expected a filesystem refusal");
	};
	assert_eq!(error.raw_os_error(), Some(libc::ENAMETOOLONG));
	assert_eq!(std::fs::read_dir(&arena.dest).unwrap().count(), 0);
}

#[test]
fn repeated_parent_pax_path_is_refused() {
	let arena = Arena::new();
	let name = "../".repeat(5000) + "outside/secret";
	let tar = archive(&[
		pax(&[("path", name.as_bytes())]),
		file("placeholder", b"pwn", 0o644),
	]);
	let result = run(&arena, &tar);
	arena.assert_outside_untouched("repeated parent PAX path");
	assert_escape(result, &name);
	assert_eq!(std::fs::read_dir(&arena.dest).unwrap().count(), 0);
}

fn assert_escape(result: crate::error::Result<()>, path: &str) {
	let ComposeError::Build(message) = result.expect_err("parent traversal must be refused") else {
		panic!("expected an archive path refusal");
	};
	assert_eq!(
		message,
		format!("cp: refusing archive entry that escapes destination: {path}")
	);
}
