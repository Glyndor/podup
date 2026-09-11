//! Every tar podup uploads is built by one constructor.
//!
//! `tar::Builder::new` turns sparse-file support on. A file with holes then
//! goes out as a GNU sparse entry, typeflag `b'S'`, decimal 83, which Podman
//! refuses: `unhandled tar header type 83` on the build endpoint,
//! `unrecognized Typeflag S` on the archive endpoint that `cp` and the watch
//! sync use. That is #1775: a build context Podman's own CLI packs and accepts
//! was one `podup build` could not upload, and a `podup cp` of the same file
//! failed too.
//!
//! Four call sites had the default, in three unrelated files, and they were
//! wrong the same way because each was written on its own. Turning the flag off
//! four times would leave the fifth site free to be written the same way again,
//! and nothing would say so until somebody with a sparse file reported it. So
//! the constructor moved to `engine::tar_stream::builder` and this is the gate
//! that keeps it the only one.
//!
//! Test code is exempt: a test that feeds an archive to the unpacking side has
//! to build that archive, and what it builds never reaches Podman.

use std::fs;
use std::path::{Path, PathBuf};

/// The single file allowed to name the crate's constructor.
const HOME: &str = "internal/engine/tar_stream.rs";

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
	for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{} is readable: {e}", dir.display()))
	{
		let path = entry.expect("entry is readable").path();
		if path.is_dir() {
			rust_sources(&path, out);
		} else if path.extension().is_some_and(|e| e == "rs") {
			out.push(path);
		}
	}
}

#[test]
fn only_tar_stream_constructs_a_tar_builder() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"));
	let mut sources = Vec::new();
	rust_sources(&root.join("internal"), &mut sources);
	assert!(
		sources.len() > 50,
		"expected to scan the whole crate, found {} files; the walk is broken \
		 and a gate that reads nothing reports nothing",
		sources.len()
	);

	let mut offenders = Vec::new();
	let mut saw_home = false;
	for path in &sources {
		let rel = path
			.strip_prefix(root)
			.expect("under the manifest dir")
			.to_string_lossy()
			.replace('\\', "/");
		let name = path.file_name().unwrap().to_string_lossy().into_owned();
		// A sibling `*_tests.rs` is test code even though it lives beside the
		// module it tests, which is the convention this crate uses.
		if name.ends_with("_tests.rs") {
			continue;
		}
		let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{rel} is readable: {e}"));
		// Comments are stripped: `tar_stream.rs` explains the defect by naming
		// the call, and the docs of a rule must not read as a breach of it.
		let code: String = text
			.lines()
			.filter(|l| !l.trim_start().starts_with("//"))
			.collect::<Vec<_>>()
			.join("\n");
		// Two spellings, because closing only the first one leaves the rule
		// looking enforced while the ordinary way to write it walks past.
		// `use tar::Builder;` followed by a bare `Builder::new(writer)` is
		// idiomatic Rust and constructs exactly the same sparse-enabled
		// builder, and a bare `Builder::new(` cannot be matched on its own:
		// `std::thread::Builder` and `std::fs::DirBuilder` are both in this
		// crate already. So the import is what gets refused, which leaves
		// `tar::Builder::new(` as the only way to reach the constructor and
		// makes that literal worth searching for.
		let imports_builder = code
			.lines()
			.map(str::trim_start)
			.any(|l| l.starts_with("use tar::") && l.contains("Builder"));
		// `use tar as t;` would put the constructor back within reach under a
		// name this test cannot predict.
		let renames_crate = code
			.lines()
			.map(str::trim_start)
			.any(|l| l.starts_with("use tar as"));
		if !code.contains("tar::Builder::new(") && !imports_builder && !renames_crate {
			continue;
		}
		if rel == HOME {
			saw_home = true;
		} else {
			offenders.push(rel);
		}
	}

	assert!(
		saw_home,
		"{HOME} no longer constructs a tar::Builder. Either it moved, and this \
		 gate has to follow it, or the crate stopped building tars and the gate \
		 is dead weight. It is not a pass either way."
	);
	assert!(
		offenders.is_empty(),
		"these files construct a tar::Builder directly instead of calling \
		 engine::tar_stream::builder, so the sparse default is back on and a \
		 sparse file will fail the transfer, with `unhandled tar header type 83` \
		 or `unrecognized Typeflag S` depending on the endpoint: {offenders:?}"
	);
}
