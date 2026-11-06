use super::*;

// --- a directory replaced by a link while the sync runs ------------------

/// The body of the entry named `path` in a gzipped tar.
fn tar_body(gz: &[u8], path: &str) -> Vec<u8> {
	let mut decoder = GzDecoder::new(gz);
	let mut raw = Vec::new();
	decoder.read_to_end(&mut raw).unwrap();
	let mut archive = tar::Archive::new(&raw[..]);
	for entry in archive.entries().unwrap() {
		let mut entry = entry.unwrap();
		if entry.path().unwrap().to_string_lossy() == path {
			let mut body = Vec::new();
			entry.read_to_end(&mut body).unwrap();
			return body;
		}
	}
	panic!("no entry named {path}");
}

/// A directory of the watched tree that is replaced by a link while the sync is
/// being packed does not redirect what goes out under its name: the sync walks
/// through descriptors, so the entry carries the file the directory held when
/// it was opened.
#[cfg(unix)]
#[test]
fn replacing_a_visited_directory_with_a_link_does_not_redirect_the_sync() {
	let dir = tempdir().unwrap();
	let outside = tempdir().unwrap();
	let tree = dir.path().canonicalize().unwrap().join("tree");
	let sub = tree.join("sub");
	fs::create_dir_all(&sub).unwrap();
	fs::write(sub.join("credencial"), b"INSIDE").unwrap();
	fs::write(outside.path().join("credencial"), b"OUTSIDE-MARKER").unwrap();

	let swap_sub = sub.clone();
	let target = outside.path().to_path_buf();
	let swapped = std::cell::Cell::new(false);
	// The filter is asked about every entry before the walk opens it. Asking for
	// the child is the moment `sub` is already open, which is what is under test.
	let skip = |name: &Path| {
		if name.ends_with("credencial") && !swapped.replace(true) {
			let moved = swap_sub.with_extension("moved");
			fs::rename(&swap_sub, &moved).unwrap();
			std::os::unix::fs::symlink(&target, &swap_sub).unwrap();
		}
		false
	};
	let (gz, _) = sync_to_gz_with_sent(&tree, Path::new("tree"), &skip).unwrap();
	assert!(swapped.get(), "the swap never happened");
	assert_eq!(tar_body(&gz, "tree/sub/credencial"), b"INSIDE");
}

/// A sink that replaces a directory with a link to another one the first time
/// the archive is written to: the header of the wrapper entry, written after the
/// root was looked at and before it is walked.
#[cfg(unix)]
struct SwapRootOnFirstWrite<W> {
	inner: W,
	swap: Option<(std::path::PathBuf, std::path::PathBuf)>,
}

#[cfg(unix)]
impl<W: std::io::Write> std::io::Write for SwapRootOnFirstWrite<W> {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
		if let Some((dir, target)) = self.swap.take() {
			fs::rename(&dir, dir.with_extension("moved"))?;
			std::os::unix::fs::symlink(&target, &dir)?;
		}
		self.inner.write(buf)
	}

	fn flush(&mut self) -> std::io::Result<()> {
		self.inner.flush()
	}
}

/// The directory of a watch event is replaced by a link between the moment it
/// was looked at and the moment it is walked. The sync refuses the link instead
/// of walking whatever it points at.
#[cfg(unix)]
#[test]
fn a_root_replaced_by_a_link_while_the_sync_starts_is_refused() {
	let dir = tempdir().unwrap();
	let outside = tempdir().unwrap();
	let tree = dir.path().canonicalize().unwrap().join("tree");
	fs::create_dir(&tree).unwrap();
	fs::write(tree.join("inside.txt"), b"INSIDE").unwrap();
	fs::write(outside.path().join("outside.txt"), b"OUTSIDE-MARKER").unwrap();

	let sink = SwapRootOnFirstWrite {
		inner: Vec::new(),
		swap: Some((tree.clone(), outside.path().to_path_buf())),
	};
	let mut tar = tar::Builder::new(GzEncoder::new(sink, Compression::default()));
	let mut sent = Vec::new();
	let result = build_sync_tar(&tree, Path::new("tree"), &mut tar, &mut sent, &|_| false);
	assert!(
		result.is_err(),
		"the sync walked a directory that had been replaced by a link"
	);
	assert!(
		!sent.iter().any(|e| e.path().contains("outside.txt")),
		"a file of the directory the link points at was recorded: {sent:?}"
	);
}

/// A directory above the root of the sync, not only the root itself, replaced by
/// a link while the sync starts: any component of the path is refused.
#[cfg(unix)]
#[test]
fn an_ancestor_replaced_by_a_link_while_the_sync_starts_is_refused() {
	let dir = tempdir().unwrap();
	let outside = tempdir().unwrap();
	let ancestor = dir.path().canonicalize().unwrap().join("a");
	let root = ancestor.join("b");
	fs::create_dir_all(&root).unwrap();
	fs::write(root.join("inside.txt"), b"INSIDE").unwrap();
	fs::create_dir_all(outside.path().join("b")).unwrap();
	fs::write(
		outside.path().join("b").join("outside.txt"),
		b"OUTSIDE-MARKER",
	)
	.unwrap();

	let sink = SwapRootOnFirstWrite {
		inner: Vec::new(),
		swap: Some((ancestor.clone(), outside.path().to_path_buf())),
	};
	let mut tar = tar::Builder::new(GzEncoder::new(sink, Compression::default()));
	let mut sent = Vec::new();
	let result = build_sync_tar(&root, Path::new("b"), &mut tar, &mut sent, &|_| false);
	assert!(
		result.is_err(),
		"the sync walked a directory reached through a replaced ancestor"
	);
	assert!(
		!sent.iter().any(|e| e.path().contains("outside.txt")),
		"a file outside the tree was recorded: {sent:?}"
	);
}
