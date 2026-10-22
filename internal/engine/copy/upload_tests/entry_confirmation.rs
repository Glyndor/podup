use super::*;

/// The error message the caller sees names the entry that failed AND the
/// stat the runtime answered, not just the verdict. This is what makes the
/// next diagnosis not blind: a log carries the path and the literal
/// `PathStat`, and an operator can act on it without re-running the upload.
#[tokio::test]
async fn a_refused_entry_message_names_the_path_and_the_stat() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());

	// One entry is stale: `payload/plain.txt` was meant to be 15 bytes and
	// arrived at 14. The path is what the user can look at; the stat is the
	// evidence the runtime said it was 14 bytes.
	let stale = "/tmp/payload/plain.txt";
	let disk: Vec<(&str, OnDisk)> = LANDED_TREE
		.into_iter()
		.map(|(p, e)| {
			if p == stale {
				(p, OnDisk::File(14))
			} else {
				(p, e)
			}
		})
		.collect();
	let fake = runtime(Put::HangsUp, &disk);

	let result = upload(&fake, &payload, "payload", None).await;
	let msg = copy_detail(result, "stale plain.txt");

	// The path of the entry that failed, relative to the extract dir, is in
	// the message. An operator looking at the log can see this entry.
	assert!(
		msg.contains("payload/plain.txt"),
		"the entry path must be named in the message: {msg}"
	);

	// The literal `PathStat` from the runtime is in the message. The Debug
	// rendering of `PathStat` carries the size and mode the runtime reported,
	// which is what the next diagnosis needs to compare expected and answered.
	assert!(
		msg.contains("size"),
		"the literal PathStat (whose Debug starts with `size`) must be in the \
		 message: {msg}"
	);
	assert!(
		msg.contains("14"),
		"the runtime's reported size (14) must be in the message: {msg}"
	);

	// The expected kind is also named, so the next diagnosis knows what the
	// upload was supposed to land.
	assert!(
		msg.contains("File(15)"),
		"the expected kind (a 15-byte file) must be in the message: {msg}"
	);

	// The actionable hint the existing message carried stays: it is true
	// (the bytes may have landed) and useful (the operator can look).
	assert!(
		msg.contains("may or may not have landed"),
		"the existing hint must stay: {msg}"
	);
}

/// A dangling symlink is packed as a link, and a link is nothing the stat
/// endpoint can be asked about. With nothing to ask, nothing is confirmed:
/// an upload is never a success for want of a question.
#[tokio::test]
async fn an_upload_with_nothing_to_ask_about_stays_unconfirmed() {
	let dir = tempfile::tempdir().unwrap();
	let link = dir.path().join("dangling");
	std::os::unix::fs::symlink("nowhere", &link).unwrap();
	let fake = runtime(Put::HangsUp, &[("/tmp", OnDisk::Dir)]);

	let result = upload(&fake, &link, "dangling", None).await;

	assert_unconfirmed(result, "a lone symlink");
}

/// The single-file answers, held where they were: confirmed on its size, and
/// refused when the entry is absent or is still the old one.
#[tokio::test]
async fn a_single_file_is_confirmed_on_its_size_and_on_nothing_less() {
	let dir = tempfile::tempdir().unwrap();
	let file = dir.path().join("f.txt");
	std::fs::write(&file, b"fifteen bytes!!").unwrap();

	let landed = runtime(Put::HangsUp, &[("/tmp/f.txt", OnDisk::File(15))]);
	let result = upload(&landed, &file, "f.txt", None).await;
	assert!(
		result.is_ok(),
		"the file is there at its size, got {result:?}"
	);

	let absent = runtime(Put::HangsUp, &[("/tmp", OnDisk::Dir)]);
	assert_unconfirmed(upload(&absent, &file, "f.txt", None).await, "absent file");

	let old = runtime(Put::HangsUp, &[("/tmp/f.txt", OnDisk::File(14))]);
	assert_unconfirmed(
		upload(&old, &file, "f.txt", None).await,
		"the previous file",
	);

	let a_directory = runtime(Put::HangsUp, &[("/tmp/f.txt", OnDisk::Dir)]);
	assert_unconfirmed(
		upload(&a_directory, &file, "f.txt", None).await,
		"a directory of that name",
	);
}
