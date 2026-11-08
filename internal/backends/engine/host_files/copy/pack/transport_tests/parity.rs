use super::*;

// --- the recorder against the archive read back ---------------------------

fn sorted(mut v: Vec<SentEntry>) -> Vec<(String, String)> {
	v.sort_by(|a, b| a.path.cmp(&b.path));
	v.into_iter()
		.map(|e| (e.path, format!("{:?}", e.kind)))
		.collect()
}

/// Drive the unified `pack_path` into a `Vec<u8>` and return both the bytes
/// and the recorded entry list. Drives the parity tests below.
fn pack_to_vec(
	src: &Path,
	follow_link: bool,
	name_override: Option<&str>,
	contents: bool,
) -> (Vec<u8>, Vec<SentEntry>) {
	let mut buf = Vec::new();
	let mut sent = Vec::new();
	{
		let mut tar = tar::Builder::new(&mut buf);
		pack_path(
			src,
			follow_link,
			name_override,
			contents,
			&mut tar,
			&mut sent,
		)
		.expect("pack_path");
	}
	(buf, sent)
}

/// Every shape the existing `sent_entries` tests already build, in one tree:
/// a file, a nested directory, a symlink, an empty file, and a directory next
/// to them. Streaming the recording and reading the archive back must agree
/// on the list, otherwise the post-PUT confirmation will diverge from what
/// the unified `pack_path` produced byte for byte.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_matches_what_pack_path_wrote_for_a_mixed_tree() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::create_dir(payload.join("empty")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"beside-the-rest").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![7u8; 1234]).unwrap();
	std::fs::write(payload.join("sub/nothing"), b"").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let (bytes, _) = pack_to_vec(&payload, false, None, false);
	let from_archive = sorted(sent_entries(&bytes).unwrap());

	let (recorded, result) = run(pack_path_stream(&payload, false, None, false, counter())).await;
	result.expect("packer should not error on a regular tree");
	let from_stream = sorted(recorded);

	assert_eq!(
		from_stream, from_archive,
		"the streaming recorder and sent_entries must agree"
	);
}

/// `contents=true` packs every descendant at its relative path with no wrapper
/// entry. The streaming recorder must produce the same list, otherwise
/// `tree_landed` would stat the wrapper that no longer exists in the archive.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_matches_what_pack_path_wrote_for_contents_mode() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::write(payload.join("a.txt"), b"hi").unwrap();
	std::fs::write(payload.join("sub/b.txt"), b"hello").unwrap();

	let (bytes, _) = pack_to_vec(&payload, false, None, true);
	let from_archive = sorted(sent_entries(&bytes).unwrap());

	let (recorded, result) = run(pack_path_stream(&payload, false, None, true, counter())).await;
	result.expect("packer should not error on contents mode");
	let from_stream = sorted(recorded);

	assert_eq!(
		from_stream, from_archive,
		"contents mode must drop the wrapper on both sides"
	);
}

/// Renaming on the way in must rename on both sides. Otherwise the
/// confirmation would stat the source's own name and find nothing.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_matches_what_pack_path_wrote_for_a_renamed_tree() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"hi").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![9u8; 64]).unwrap();

	let (bytes, _) = pack_to_vec(&payload, false, Some("renamed"), false);
	let from_archive: Vec<String> = sorted(sent_entries(&bytes).unwrap())
		.into_iter()
		.map(|(p, _)| p)
		.collect();

	let (recorded, result) = run(pack_path_stream(
		&payload,
		false,
		Some("renamed"),
		false,
		counter(),
	))
	.await;
	result.expect("packer should not error on a renamed tree");
	let from_stream: Vec<String> = sorted(recorded).into_iter().map(|(p, _)| p).collect();

	assert_eq!(
		from_stream, from_archive,
		"the rename must apply on both sides"
	);
}

/// A lone symlink: the recording carries exactly one `Link` entry, mirroring
/// `sent_entries` on the same source. Without this pin, dropping symlinks
/// from the recorder would silently pass — the archive would still carry
/// the link, but the post-PUT confirmation would never ask about it.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_includes_a_lone_symlink() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();

	let (bytes, _) = pack_to_vec(&link, false, None, false);
	let from_archive = sent_entries(&bytes).unwrap();

	let (recorded, result) = run(pack_path_stream(&link, false, None, false, counter())).await;
	result.expect("packer should not error on a lone symlink");

	assert_eq!(
		sorted(recorded.clone()),
		sorted(from_archive.clone()),
		"the streaming recorder must include the symlink the archive carries"
	);
	assert!(
		recorded
			.iter()
			.any(|e| matches!(&e.kind, SentKind::Link(t) if t == "nowhere")),
		"the recorded list must carry the link target the destination is asked about, got {:?}",
		recorded
	);
}

/// A single file: one `File(size)` entry, parity with the archive read-back.
#[cfg(unix)]
#[tokio::test(flavor = "current_thread")]
async fn streaming_recording_matches_what_pack_path_wrote_for_a_single_file() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("f.txt");
	std::fs::write(&file, b"fifteen bytes!!").unwrap();

	let (bytes, _) = pack_to_vec(&file, false, None, false);
	let from_archive = sorted(sent_entries(&bytes).unwrap());

	let (recorded, result) = run(pack_path_stream(&file, false, None, false, counter())).await;
	result.expect("packer should not error on a single file");
	let from_stream = sorted(recorded);

	assert_eq!(from_stream, from_archive);
}

/// The single-call parity: pack to a `Vec` with the recorder on, then
/// assert `recorded == sent_entries(&vec)`. The recorder and the bytes must
/// agree on every entry; this pins the contract that the recorder and the
/// archive are the same list. It also pins the symmetric shape: if the
/// recording side adds an entry the tar does not carry, the parity fails
/// on this side too.
#[cfg(unix)]
#[test]
fn the_recorder_agrees_with_what_sent_entries_reads_back() {
	let dir = tempfile::tempdir().unwrap();
	let payload = dir.path().join("payload");
	std::fs::create_dir_all(payload.join("sub")).unwrap();
	std::fs::create_dir(payload.join("empty")).unwrap();
	std::fs::write(payload.join("plain.txt"), b"beside-the-rest").unwrap();
	std::fs::write(payload.join("sub/inner.bin"), vec![7u8; 1234]).unwrap();
	std::fs::write(payload.join("sub/nothing"), b"").unwrap();
	std::os::unix::fs::symlink("nowhere", payload.join("dangling")).unwrap();

	let (bytes, recorded) = pack_to_vec(&payload, false, None, false);
	let from_archive = sent_entries(&bytes).expect("sent_entries");

	let mut a: Vec<(String, String)> = recorded
		.iter()
		.map(|e| (e.path.clone(), format!("{:?}", e.kind)))
		.collect();
	let mut b: Vec<(String, String)> = from_archive
		.iter()
		.map(|e| (e.path.clone(), format!("{:?}", e.kind)))
		.collect();
	a.sort();
	b.sort();
	assert_eq!(
		a, b,
		"the recorder must equal sent_entries of the same bytes"
	);
}
