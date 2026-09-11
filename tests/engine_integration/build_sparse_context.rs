//! A build context holding a file with holes, uploaded to a live Podman.
//!
//! `internal/engine/tar_stream.rs` keeps GNU sparse entries out of the archive
//! and `tests/tar_builder_single_site.rs` keeps the constructor single, but
//! both read bytes we wrote ourselves. Neither asks Podman. This one does: it
//! is the only test that would have caught #1775 as the user met it, an
//! `unhandled tar header type 83` from the far end of the socket.
use super::*;

use std::io::{Seek, SeekFrom, Write};

/// Write `hole` bytes of nothing followed by `tail`, and report whether the
/// filesystem left the range unallocated.
///
/// `set_len` does not move the cursor, so the seek is what puts the tail at the
/// end instead of at offset 0. Without a real hole the upload below carries an
/// ordinary file and proves nothing, so the caller checks this before drawing
/// any conclusion from a pass.
fn write_sparse(path: &std::path::Path, hole: u64, tail: &[u8]) -> bool {
	let mut f = fs::File::create(path).unwrap();
	f.set_len(hole).unwrap();
	f.seek(SeekFrom::Start(hole)).unwrap();
	f.write_all(tail).unwrap();
	f.sync_all().unwrap();
	drop(f);
	#[cfg(unix)]
	{
		use std::os::unix::fs::MetadataExt;
		let meta = fs::metadata(path).unwrap();
		meta.blocks() * 512 < meta.size()
	}
	#[cfg(not(unix))]
	{
		false
	}
}

const HOLE: u64 = 1 << 20;
const TAIL: &[u8] = b"tail-marker";

#[tokio::test]
async fn build_uploads_a_context_that_holds_a_sparse_file() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let sparse = dir.path().join("holey.bin");
	if !write_sparse(&sparse, HOLE, TAIL) {
		// The filesystem stored the zeroes, so the context carries an ordinary
		// file and a green result would say nothing about sparse handling.
		eprintln!("build_sparse_context: no hole on this filesystem, nothing measured");
		return;
	}
	// `COPY` the file in and read its size back from inside the image: an upload
	// that succeeded but truncated the expanded holes would pass a check that
	// only asked whether the build returned Ok.
	fs::write(
		dir.path().join("Dockerfile"),
		b"FROM alpine:latest\nCOPY holey.bin /holey.bin\nRUN stat -c %s /holey.bin > /size\n",
	)
	.unwrap();

	let proj = proj("sparsectx");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let image_tag = format!("podup-test-sparsectx-{}:latest", std::process::id());
	let yaml = format!(
		"services:\n  app:\n    build:\n      context: .\n    image: {image_tag}\n    command: [\"sleep\", \"infinity\"]\n"
	);
	let file = parse_str(&yaml).unwrap();

	let built = engine.up(&file).await;
	let size = if built.is_ok() {
		engine
			.test_exec_capture(&format!("{proj}-app-1"), vec!["cat".into(), "/size".into()])
			.await
			.unwrap_or_default()
	} else {
		String::new()
	};
	let _ = engine.down(&file).await;
	let _ = std::process::Command::new("podman")
		.args(["rmi", "-f", &image_tag])
		.status();

	// The failure this guards against is server-side and reads
	// `unhandled tar header type 83`, so the error text is worth printing: a
	// different failure here is a different bug and should not be read as this
	// one coming back.
	built.unwrap_or_else(|e| {
		panic!("a build context holding a sparse file must upload; podman said: {e}")
	});
	assert_eq!(
		size.trim(),
		(HOLE + TAIL.len() as u64).to_string(),
		"the file arrived with the wrong length, so the holes were not expanded intact"
	);
}
