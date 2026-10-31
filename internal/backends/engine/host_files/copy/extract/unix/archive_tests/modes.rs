use std::os::unix::fs::PermissionsExt;

use super::super::super::hostile_support::{archive, dir, file};
use super::super::{extract, Directory};

#[cfg(unix)]
#[test]
fn missing_destination_flattens_a_readonly_wrapper_before_finalizing_modes() {
	assert_missing_destination_flattens_wrapper(0o555);
}

#[cfg(unix)]
#[test]
fn missing_destination_flattens_a_zero_mode_wrapper_before_finalizing_modes() {
	assert_missing_destination_flattens_wrapper(0o000);
}

#[cfg(unix)]
fn assert_missing_destination_flattens_wrapper(wrapper_mode: u32) {
	let tmp = tempfile::tempdir().unwrap();
	let dst = tmp.path().join("dest");
	assert!(!dst.exists());
	let bytes = archive(&[
		dir("datos/", wrapper_mode),
		file("datos/contenido", b"content", 0o644),
		dir("datos/sub/", 0o500),
		file("datos/sub/nested", b"nested content", 0o600),
	]);
	super::super::super::archive::extract_archive(&bytes, &dst).unwrap();
	let content = std::fs::read(dst.join("contenido"));
	let nested = std::fs::read(dst.join("sub/nested"));
	let sub_mode = std::fs::metadata(dst.join("sub"))
		.unwrap()
		.permissions()
		.mode()
		& 0o7777;
	std::fs::set_permissions(dst.join("sub"), std::fs::Permissions::from_mode(0o700)).unwrap();
	assert!(!dst.join("datos").exists());
	assert_eq!(content.unwrap(), b"content");
	assert_eq!(nested.unwrap(), b"nested content");
	assert_eq!(sub_mode, 0o500);
	std::fs::write(dst.join("additional"), b"destination remains writable").unwrap();
}

#[cfg(unix)]
#[test]
fn readonly_directories_receive_their_final_mode_after_their_contents() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let bytes = archive(&[
		dir("datos/", 0o555),
		file("datos/contenido", b"content", 0o644),
	]);
	let result = extract(&bytes[..], &root);
	let metadata = std::fs::metadata(tmp.path().join("datos")).unwrap();
	let content = std::fs::read(tmp.path().join("datos/contenido"));
	std::fs::set_permissions(
		tmp.path().join("datos"),
		std::fs::Permissions::from_mode(0o755),
	)
	.unwrap();
	result.unwrap();
	assert_eq!(metadata.permissions().mode() & 0o7777, 0o555);
	assert_eq!(content.unwrap(), b"content");
}

#[cfg(unix)]
#[test]
fn repeated_directory_headers_keep_the_last_mode_with_nested_contents() {
	let tmp = tempfile::tempdir().unwrap();
	let root = Directory::open_host(tmp.path()).unwrap();
	let bytes = archive(&[
		dir("datos/sub/", 0o500),
		dir("datos/", 0o555),
		file("datos/sub/first", b"first", 0o600),
		dir("datos/sub/", 0o555),
		dir("datos/", 0o500),
		file("datos/sub/second", b"second", 0o644),
	]);
	let result = extract(&bytes[..], &root);
	let outer = std::fs::metadata(tmp.path().join("datos")).unwrap();
	let inner = std::fs::metadata(tmp.path().join("datos/sub")).unwrap();
	let content = std::fs::read(tmp.path().join("datos/sub/second"));
	for path in ["datos", "datos/sub"] {
		std::fs::set_permissions(
			tmp.path().join(path),
			std::fs::Permissions::from_mode(0o755),
		)
		.unwrap();
	}
	result.unwrap();
	assert_eq!(outer.permissions().mode() & 0o7777, 0o500);
	assert_eq!(inner.permissions().mode() & 0o7777, 0o555);
	assert_eq!(content.unwrap(), b"second");
}
