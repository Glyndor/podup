use super::*;

/// #1777. The copy landed, the runtime hung up, and the caller was told it
/// failed, because a directory gave the confirmation nothing to compare.
#[tokio::test]
async fn a_directory_that_landed_is_confirmed_when_the_runtime_hangs_up() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let fake = runtime(Put::HangsUp, &LANDED_TREE);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(
		result.is_ok(),
		"every entry of the tree is at the destination, got {result:?}"
	);
}

/// The other half, and the one that matters more: hanging up is not evidence.
/// Nothing of the payload is in the container, so nothing may be confirmed.
#[tokio::test]
async fn a_directory_that_did_not_land_is_still_a_failure() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let fake = runtime(Put::HangsUp, &[("/tmp", OnDisk::Dir)]);

	let result = upload(&fake, &payload, "payload", None).await;

	assert_unconfirmed(result, "empty destination");
}

/// A stream cut part of the way leaves some of the tree behind. Take each entry
/// away in turn, with whatever lived under it, and the upload must be refused
/// every time: confirming on the top directory, or on any one child, would
/// pass some of these.
#[tokio::test]
async fn a_directory_that_landed_in_part_is_a_failure_whichever_part_is_missing() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());

	for (missing, _) in LANDED_TREE {
		let partial: Vec<(&str, OnDisk)> = LANDED_TREE
			.into_iter()
			.filter(|(path, _)| *path != missing && !path.starts_with(&format!("{missing}/")))
			.collect();
		let fake = runtime(Put::HangsUp, &partial);

		let result = upload(&fake, &payload, "payload", None).await;

		assert_unconfirmed(result, &format!("without {missing}"));
	}
}

/// Present is not the same as arrived: an entry left over from before the
/// upload, at another size or of another kind, is not what was sent.
#[tokio::test]
async fn a_directory_whose_entries_do_not_match_what_was_sent_is_a_failure() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let stale: [(&str, OnDisk, &str); 3] = [
		(
			"/tmp/payload/plain.txt",
			OnDisk::File(14),
			"a file one byte short",
		),
		(
			"/tmp/payload/sub/inner.bin",
			OnDisk::File(0),
			"a file that is empty",
		),
		(
			"/tmp/payload/empty",
			OnDisk::File(0),
			"a file where a directory was sent",
		),
	];

	for (path, instead, case) in stale {
		let disk: Vec<(&str, OnDisk)> = LANDED_TREE
			.iter()
			.map(|(p, e)| {
				if *p == path {
					(*p, instead.clone())
				} else {
					(*p, e.clone())
				}
			})
			.collect();
		let fake = runtime(Put::HangsUp, &disk);

		let result = upload(&fake, &payload, "payload", None).await;

		assert_unconfirmed(result, case);
	}
}

/// A stat the runtime would not answer is not an answer. Everything else of the
/// tree is in place, so only the unreadable entry can be what refuses it.
#[tokio::test]
async fn a_directory_with_an_entry_that_cannot_be_read_back_is_a_failure() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());

	for (unreadable, _) in LANDED_TREE {
		let disk: Vec<(&str, OnDisk)> = LANDED_TREE
			.into_iter()
			.map(|(p, e)| {
				(
					p,
					if p == unreadable {
						OnDisk::Unreadable
					} else {
						e
					},
				)
			})
			.collect();
		let fake = runtime(Put::HangsUp, &disk);

		let result = upload(&fake, &payload, "payload", None).await;

		assert_unconfirmed(result, &format!("{unreadable} answers 500"));
	}
}

/// `cp dir svc:/tmp/renamed` packs the tree under the new name, and that name
/// is where it has to be looked for.
#[tokio::test]
async fn a_renamed_directory_is_confirmed_under_its_new_name() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let renamed: Vec<(String, OnDisk)> = LANDED_TREE
		.into_iter()
		.map(|(p, e)| (p.replacen("/tmp/payload", "/tmp/renamed", 1), e.clone()))
		.collect();
	let renamed: Vec<(&str, OnDisk)> = renamed
		.iter()
		.map(|(p, e)| (p.as_str(), e.clone()))
		.collect();

	let landed = runtime(Put::HangsUp, &renamed);
	let result = upload(&landed, &payload, "renamed", Some("renamed")).await;
	assert!(
		result.is_ok(),
		"the tree is there as `renamed`, got {result:?}"
	);

	// Under the source's own name it is somebody else's tree.
	let elsewhere = runtime(Put::HangsUp, &LANDED_TREE);
	let result = upload(&elsewhere, &payload, "renamed", Some("renamed")).await;
	assert_unconfirmed(result, "only the old name exists");
}
