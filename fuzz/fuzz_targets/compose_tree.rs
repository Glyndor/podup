#![no_main]

//! Fuzz the file-level compose loader on a small tree of files that refer to
//! each other.
//!
//! `parse_compose` hands the parser one string, so `include:`, `extends.file`,
//! `env_file`, `.env` and the several-`-f` merge are never reached by it. They
//! are where the interpolation budget of the whole project and the include and
//! extends caps live. This target builds up to six compose files and two env
//! files in a scratch directory from the fuzzer's bytes, wires them together
//! (cycles and missing files included), and loads the root through the public
//! file API. Any `Err` is a refusal and not a finding. A panic, a hang or a
//! run past `-rss_limit_mb` is.
//!
//! Every choice is one byte reduced into a small range, and an input that runs
//! out reads as zeros, so a seed can be written by hand: see
//! `fuzz/seeds/compose_tree/`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use libfuzzer_sys::arbitrary::{Result, Unstructured};
use libfuzzer_sys::fuzz_target;

const MAX_FILES: u8 = 6;
/// What `BIG` expands to, by class: none, 1 KiB, 64 KiB, 1 MiB.
const BIG_SIZES: [usize; 4] = [0, 1024, 64 * 1024, 1024 * 1024];
const VARS: [&str; 4] = ["${BIG}", "${SMALL}", "${UNSET}", "${BIG:-fallback}"];

/// Per-iteration scratch directory, removed on drop.
struct ScratchDir {
	path: PathBuf,
}

impl ScratchDir {
	fn new() -> std::io::Result<Self> {
		static COUNTER: AtomicU64 = AtomicU64::new(0);
		let n = COUNTER.fetch_add(1, Ordering::Relaxed);
		let path = std::env::temp_dir().join(format!(
			"podup-fuzz-compose-tree-{}-{n}",
			std::process::id()
		));
		std::fs::create_dir_all(&path)?;
		Ok(Self { path })
	}
}

impl Drop for ScratchDir {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.path);
	}
}

/// One byte reduced into `0..count`.
fn pick(u: &mut Unstructured<'_>, count: u8) -> Result<u8> {
	u.int_in_range(0..=count - 1)
}

fn env_text(class: u8) -> String {
	format!("BIG={}\nSMALL=s\n", "x".repeat(BIG_SIZES[class as usize]))
}

/// The text of one compose file in a tree of `files`.
fn compose_file(u: &mut Unstructured<'_>, files: u8) -> Result<String> {
	let mut out = String::new();
	let includes = pick(u, 4)?;
	if includes > 0 {
		let mut targets = Vec::new();
		for _ in 0..includes {
			targets.push(format!("f{}.yaml", pick(u, files + 1)?));
		}
		let _ = writeln!(out, "include: [{}]", targets.join(", "));
	}
	let services = pick(u, 4)?;
	if services > 0 {
		out.push_str("services:\n");
	}
	for service in 0..services {
		let refs = pick(u, 21)?;
		let var = VARS[pick(u, 4)? as usize];
		let _ = writeln!(out, "  s{service}:\n    image: alpine");
		if refs > 0 {
			let list = vec![format!("\"{var}\""); refs as usize].join(", ");
			let _ = writeln!(out, "    command: [{list}]");
		}
		match pick(u, 3)? {
			1 => out.push_str("    extends: {service: s0}\n"),
			2 => {
				let file = pick(u, files + 1)?;
				let _ = writeln!(out, "    extends: {{file: f{file}.yaml, service: s0}}");
			}
			_ => {}
		}
		if pick(u, 2)? == 1 {
			let _ = writeln!(out, "    env_file: [e{}.env]", pick(u, 2)?);
		}
	}
	let length = pick(u, 33)? as usize;
	let tail = u.bytes(length)?;
	out.push_str(&String::from_utf8_lossy(tail));
	Ok(out)
}

/// Write the tree and return the files to load, root first.
fn build(u: &mut Unstructured<'_>, dir: &Path) -> Result<(Vec<PathBuf>, bool, u8)> {
	let files = 1 + pick(u, MAX_FILES)?;
	let entry = pick(u, 3)?;
	let interpolate = pick(u, 2)? == 0;
	let env_file_class = pick(u, 4)?;
	std::fs::write(dir.join(".env"), env_text(pick(u, 4)?)).ok();
	for index in 0..2 {
		std::fs::write(dir.join(format!("e{index}.env")), env_text(pick(u, 4)?)).ok();
	}
	let mut paths = Vec::new();
	for index in 0..files {
		let path = dir.join(format!("f{index}.yaml"));
		std::fs::write(&path, compose_file(u, files)?).ok();
		paths.push(path);
	}
	paths.truncate(if entry == 2 { files as usize } else { 1 });
	Ok((paths, interpolate, env_file_class))
}

fuzz_target!(|data: &[u8]| {
	let Ok(scratch) = ScratchDir::new() else {
		return;
	};
	let mut u = Unstructured::new(data);
	let Ok((paths, interpolate, class)) = build(&mut u, &scratch.path) else {
		return;
	};
	// Every route a command takes: the one-file entry points, with and without
	// interpolation, and the list of files with an `--env-file` among them.
	let env_files: Vec<String> = if class == 0 {
		Vec::new()
	} else {
		vec![scratch.path.join("e0.env").to_string_lossy().into_owned()]
	};
	let _ = match paths.len() {
		1 if interpolate => podup::compose::parse_file_with_env_files(&paths[0], &env_files),
		1 => podup::compose::parse_file_with_env_files_interp(&paths[0], &env_files, false),
		_ => podup::compose::parse_files_with_env_files_interp(&paths, &env_files, interpolate),
	};
});
