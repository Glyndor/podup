//! Hostile header fields: modes, special files, names, sizes, PAX records and
//! truncated or garbled archives, all handed to the container-to-host
//! extractor.

use std::os::unix::fs::MetadataExt;

use tar::EntryType;

use super::archive::extract_tar_guarded;
use super::hostile_support::{archive, dir, file, pax, raw, set_mode, symlink, Arena};

fn run(arena: &Arena, bytes: &[u8]) -> crate::error::Result<()> {
	extract_tar_guarded(bytes, &arena.dest)
}

#[test]
fn file_modes_lose_setuid_setgid_sticky_and_group_other_write() {
	for mode in [0o4755, 0o2755, 0o6777, 0o1777, 0o7777, 0o666, 0o020] {
		let arena = Arena::new();
		run(&arena, &archive(&[file("f", b"x", mode)])).expect("extract");
		let got = std::fs::metadata(arena.dest.join("f"))
			.expect("stat")
			.mode();
		assert_eq!(
			got & 0o7022,
			0,
			"file mode {mode:o} became {:o}",
			got & 0o7777
		);
	}
}

#[test]
fn directory_modes_lose_setgid_sticky_and_group_other_write() {
	for mode in [0o2755, 0o1777, 0o777, 0o775, 0o7777] {
		let arena = Arena::new();
		run(&arena, &archive(&[dir("d/", mode)])).expect("extract");
		let got = std::fs::metadata(arena.dest.join("d"))
			.expect("stat")
			.mode();
		assert_eq!(
			got & 0o7022,
			0,
			"dir mode {mode:o} became {:o}",
			got & 0o7777
		);
	}
}

#[test]
fn device_nodes_and_fifos_never_become_special_files_on_the_host() {
	let arena = Arena::new();
	let tar = archive(&[
		raw(b"null", EntryType::Char, b"", 0o666, b""),
		raw(b"disk", EntryType::Block, b"", 0o666, b""),
		raw(b"pipe", EntryType::Fifo, b"", 0o666, b""),
	]);
	let _ = run(&arena, &tar);
	// Whatever the extractor does with these, it must not leave a device node
	// or a pipe that would block the next reader of the directory.
	for name in ["null", "disk", "pipe"] {
		if let Ok(meta) = std::fs::symlink_metadata(arena.dest.join(name)) {
			assert!(meta.file_type().is_file(), "{name} is not a plain file");
		}
	}
	arena.assert_outside_untouched("special files");
}

#[test]
fn a_nul_inside_a_name_cannot_hide_a_traversal() {
	let arena = Arena::new();
	let tar = archive(&[
		raw(
			b"ok\0/../../outside/pwn",
			EntryType::Regular,
			b"",
			0o644,
			b"pwn",
		),
		pax(&[("path", b"fine\0/../../outside/pwn")]),
		file("placeholder", b"pwn", 0o644),
	]);
	let err = run(&arena, &tar).expect_err("the PAX path escapes and must be refused");
	assert!(err.to_string().contains("escapes destination"), "{err}");
	assert!(
		arena.dest.join("ok").is_file(),
		"the part before the NUL is the name"
	);
	arena.assert_outside_untouched("NUL in names");
}

#[test]
fn pax_path_and_linkpath_cannot_smuggle_a_traversal_past_the_header_name() {
	let arena = Arena::new();
	let out = arena.outside_str();
	let tar = archive(&[
		pax(&[("path", b"../outside/pwn")]),
		file("innocent", b"pwn", 0o644),
		pax(&[("path", format!("{out}/secret").as_bytes())]),
		file("innocent2", b"pwn", 0o666),
		pax(&[("linkpath", out.as_bytes())]),
		symlink("lnk", "x"),
		file("lnk/pwn", b"pwn", 0o644),
	]);
	let _ = run(&arena, &tar);
	arena.assert_outside_untouched("pax path override");
}

#[test]
fn malformed_pax_records_end_in_an_error_or_a_skip_never_a_panic() {
	let bodies: [&[u8]; 7] = [
		b"999999999 path=../outside/pwn\n",
		b"5 path\n",
		b"0 path=x\n",
		b"-5 path=x\n",
		b"12 path=../outside/pwn",
		b"18446744073709551616 path=x\n",
		b"\xff\xfe\xfd\n",
	];
	for body in bodies {
		let arena = Arena::new();
		let hdr = raw(b"PaxHeader/x", EntryType::XHeader, b"", 0o644, body);
		run(&arena, &archive(&[hdr, file("f", b"x", 0o644)])).expect("a bad record is skipped");
		let names: Vec<_> = std::fs::read_dir(&arena.dest)
			.expect("read_dir")
			.map(|e| e.expect("entry").file_name())
			.collect();
		assert_eq!(names, ["f"], "pax body {body:?} renamed or added an entry");
		arena.assert_outside_untouched(&format!("pax body {body:?}"));
	}
}

#[test]
fn very_long_and_very_deep_names_are_handled() {
	let arena = Arena::new();
	let long = "a".repeat(300);
	let deep = "d/".repeat(3000) + "f";
	let tar = archive(&[
		pax(&[("path", long.as_bytes())]),
		file("x", b"x", 0o644),
		pax(&[("path", deep.as_bytes())]),
		file("y", b"y", 0o644),
		pax(&[("path", "../".repeat(5000).as_bytes())]),
		file("z", b"z", 0o644),
	]);
	assert!(
		run(&arena, &tar).is_err(),
		"a 300 byte component cannot be created"
	);
	arena.assert_outside_untouched("long and deep names");
}

#[test]
fn a_declared_size_far_beyond_the_body_is_an_error_and_not_an_allocation() {
	let arena = Arena::new();
	let mut header = tar::Header::new_gnu();
	header.set_size(1 << 45);
	header.set_mode(0o644);
	header.set_entry_type(EntryType::Regular);
	header.as_gnu_mut().expect("gnu").name[..1].copy_from_slice(b"f");
	header.set_cksum();
	let mut bytes = header.as_bytes().to_vec();
	bytes.extend_from_slice(&[7u8; 4096]);
	assert!(
		run(&arena, &bytes).is_err(),
		"a truncated body was accepted"
	);
}

#[test]
fn a_sparse_entry_claiming_a_huge_real_size_does_not_materialise_it() {
	let arena = Arena::new();
	let tar = archive(&[
		pax(&[
			("GNU.sparse.realsize", b"1099511627776"),
			("GNU.sparse.name", b"big"),
		]),
		file("big", b"tiny", 0o644),
		raw(b"gnusparse", EntryType::GNUSparse, b"", 0o644, b"tiny"),
	]);
	let _ = run(&arena, &tar);
	let mut total = 0;
	for entry in std::fs::read_dir(&arena.dest).expect("read_dir") {
		total += std::fs::metadata(entry.expect("entry").path())
			.expect("stat")
			.len();
	}
	assert!(total < 1 << 20, "{total} bytes appeared from a 4 byte body");
}

#[test]
fn garbage_and_truncated_archives_return_without_panicking() {
	let arena = Arena::new();
	let good = archive(&[file("f", b"hello", 0o644)]);
	let mut bad_sum = good.clone();
	bad_sum[148..156].copy_from_slice(b"0000000\0");
	let inputs: Vec<Vec<u8>> = vec![
		Vec::new(),
		vec![0u8; 511],
		vec![0xffu8; 4096],
		good[..300].to_vec(),
		good[..520].to_vec(),
		bad_sum,
		b"ustar\0".repeat(200),
	];
	for input in inputs {
		let _ = run(&arena, &input);
	}
	arena.assert_outside_untouched("garbage archives");
}

#[test]
fn an_unreadable_directory_entry_stops_the_copy_and_nothing_leaves_the_destination() {
	let arena = Arena::new();
	let tar = archive(&[
		dir("locked/", 0o000),
		file("locked/inside", b"x", 0o644),
		file("after", b"x", 0o644),
	]);
	let result = run(&arena, &tar);
	// Root keeps CAP_DAC_OVERRIDE and writes into the directory anyway, so
	// the precondition is checked on the process, not guessed from the uid.
	let probe = std::fs::File::create(arena.dest.join("locked/probe"));
	set_mode(&arena.dest.join("locked"), 0o700);
	if probe.is_err() {
		assert!(result.is_err(), "a write into a mode 000 directory passed");
	}
	arena.assert_outside_untouched("mode 000 directory");
}

#[test]
fn a_flood_of_entries_finishes_within_a_budget() {
	let arena = Arena::new();
	let parts: Vec<Vec<u8>> = (0..20_000)
		.map(|i| file(&format!("f{i}"), b"", 0o644))
		.collect();
	let start = std::time::Instant::now();
	run(&arena, &archive(&parts)).expect("extract");
	assert!(start.elapsed() < std::time::Duration::from_secs(60));
}

#[test]
fn an_entry_naming_the_destination_itself_cannot_re_mode_it() {
	for name in ["./", ".", "./."] {
		for mode in [0o777u32, 0o000, 0o4777] {
			let arena = Arena::new();
			set_mode(&arena.dest, 0o700);
			run(&arena, &archive(&[dir(name, mode)])).expect("extract");
			let got = std::fs::metadata(&arena.dest).expect("stat").mode() & 0o7777;
			set_mode(&arena.dest, 0o700);
			assert_eq!(
				got, 0o700,
				"entry {name:?} mode {mode:o} changed the destination"
			);
		}
	}
}
