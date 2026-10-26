//! What the unit writer does with whatever already sits at a unit's name. The
//! cases use several extensions on purpose: the writer is shared by every kind
//! of unit, timers and services included.

use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use super::unit_file::{inspect_existing, Existing};
use super::{write_units, QuadletUnit};

const NAMES: &[&str] = &[
	"app-web.container",
	"app-web.network",
	"app-web.volume",
	"app-web.build",
	"app-nightly-1.timer",
	"app-nightly-1.service",
];

fn unit(filename: &str) -> QuadletUnit {
	QuadletUnit {
		filename: filename.to_string(),
		contents: "# podup-owner: app\n[Container]\nImage=alpine\n".to_string(),
	}
}

fn mode(path: &Path) -> u32 {
	std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[test]
fn a_link_on_a_unit_name_is_replaced_not_written_through() {
	for name in NAMES {
		let root = tempfile::tempdir().unwrap();
		let victim = root.path().join("victim");
		std::fs::write(&victim, "precious").unwrap();
		let out = root.path().join("out");
		std::fs::create_dir(&out).unwrap();
		symlink(&victim, out.join(name)).unwrap();

		write_units(&out, &[unit(name)]).unwrap();

		assert_eq!(std::fs::read(&victim).unwrap(), b"precious", "{name}");
		let written = out.join(name);
		assert!(!written.symlink_metadata().unwrap().file_type().is_symlink());
		assert!(std::fs::read_to_string(&written)
			.unwrap()
			.contains("Image=alpine"));
		assert_eq!(mode(&written), 0o600, "{name}");
	}
}

#[test]
fn a_dangling_link_does_not_make_the_writer_create_its_target() {
	let root = tempfile::tempdir().unwrap();
	let out = root.path().join("out");
	std::fs::create_dir(&out).unwrap();
	let target = root.path().join("not-yet");
	symlink(&target, out.join("app-web.container")).unwrap();

	write_units(&out, &[unit("app-web.container")]).unwrap();

	assert!(!target.exists());
}

#[test]
fn a_hard_link_to_another_file_is_not_written_through() {
	let root = tempfile::tempdir().unwrap();
	let victim = root.path().join("victim");
	std::fs::write(&victim, "precious").unwrap();
	let out = root.path().join("out");
	std::fs::create_dir(&out).unwrap();
	std::fs::hard_link(&victim, out.join("app-web.container")).unwrap();

	write_units(&out, &[unit("app-web.container")]).unwrap();

	assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
}

#[test]
fn a_fifo_on_a_unit_name_is_refused_without_blocking() {
	let root = tempfile::tempdir().unwrap();
	let out = root.path().join("out");
	std::fs::create_dir(&out).unwrap();
	let fifo = out.join("app-web.container");
	assert!(std::process::Command::new("mkfifo")
		.arg(&fifo)
		.status()
		.unwrap()
		.success());

	let (tx, rx) = mpsc::channel();
	let dir = out.clone();
	std::thread::spawn(move || {
		let _ = tx.send(write_units(&dir, &[unit("app-web.container")]).is_err());
	});
	let refused = rx.recv_timeout(Duration::from_secs(5));
	// Free a reader stuck on the FIFO so the thread does not outlive the test.
	// Non-blocking: with no reader waiting, a plain write open would itself block.
	{
		use std::os::unix::fs::OpenOptionsExt;
		let _ = std::fs::OpenOptions::new()
			.write(true)
			.custom_flags(libc::O_NONBLOCK)
			.open(&fifo);
	}
	assert_eq!(
		refused,
		Ok(true),
		"the writer blocked on a FIFO or accepted it"
	);
}

#[test]
fn a_directory_on_a_unit_name_is_refused() {
	let root = tempfile::tempdir().unwrap();
	std::fs::create_dir(root.path().join("app-web.container")).unwrap();
	assert!(write_units(root.path(), &[unit("app-web.container")]).is_err());
}

#[test]
fn an_oversized_existing_file_is_not_read_into_memory() {
	let root = tempfile::tempdir().unwrap();
	let big = root.path().join("big.container");
	let file = std::fs::File::create(&big).unwrap();
	// Sparse: costs no disk, and an unbounded read would build a string this long.
	file.set_len(crate::filesystem::MAX_FILE_BYTES + 4096)
		.unwrap();

	match inspect_existing(&big).unwrap() {
		Existing::File(text) => assert!(text.is_none(), "the whole file was read"),
		_ => panic!("a regular file was not reported as one"),
	}
	write_units(root.path(), &[unit("big.container")]).unwrap();
	assert!(std::fs::metadata(&big).unwrap().len() < 1024);
}

#[test]
fn a_write_leaves_only_the_units_in_the_directory() {
	let root = tempfile::tempdir().unwrap();
	let units: Vec<QuadletUnit> = NAMES.iter().map(|n| unit(n)).collect();
	write_units(root.path(), &units).unwrap();
	write_units(root.path(), &units).unwrap();
	let mut found: Vec<String> = std::fs::read_dir(root.path())
		.unwrap()
		.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
		.collect();
	found.sort();
	let mut want: Vec<String> = NAMES.iter().map(|n| n.to_string()).collect();
	want.sort();
	assert_eq!(found, want);
}
