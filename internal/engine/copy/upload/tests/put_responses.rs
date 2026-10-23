use super::*;

/// Only the hang-up is recoverable. A runtime that answered with an error said
/// what happened, and a destination that looks right does not overrule it.
#[tokio::test]
async fn an_answered_refusal_is_not_recovered_by_a_matching_destination() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let fake = runtime(Put::Answers(500), &LANDED_TREE);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(
		matches!(result, Err(ComposeError::Podman(_))),
		"the runtime's own error must reach the caller, got {result:?}"
	);
}

/// Podman 5 answers the PUT, and that answer is the confirmation: nothing is
/// read back, so a tree costs it no extra requests.
#[tokio::test]
async fn an_answered_upload_is_not_read_back() {
	let dir = tempfile::tempdir().unwrap();
	let payload = payload_tree(dir.path());
	let fake = runtime(Put::Answers(200), &[]);

	let result = upload(&fake, &payload, "payload", None).await;

	assert!(result.is_ok(), "a 200 is success, got {result:?}");
	let requests = fake.requests.lock().unwrap().clone();
	assert_eq!(
		requests.len(),
		1,
		"one PUT and nothing else, got {requests:?}"
	);
	assert!(requests[0].starts_with("PUT "), "got {requests:?}");
}

/// The libpod `/archive` PUT defaults `copyUIDGID` to true, which makes
/// a host file copied into a container take the container's runtime
/// UID/GID (i.e. `0:0`). The Docker compat handler defaulted it to
/// false, which preserved the host UID/GID on the destination file
/// (as measured with podup 5.10.0 on 2026-09-24: `1000:1000`). The compensation pins the
/// docker-compat default: the PUT query must include
/// `copyUIDGID=false`, and the docker-side key (`copyUIDGID=true`)
/// must not appear. The URL helper is pinned separately in
/// `engine::lifecycle::libpod_endpoint_query_tests`; this is the
/// end-to-end wire check the streaming packer drives.
#[tokio::test]
async fn upload_carries_copy_uid_gid_false() {
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("hello.txt");
	std::fs::write(&src, b"hi").unwrap();

	// A clean 200 from the PUT (Podman 5) is enough: the upload is the
	// path under test, not the apply-then-close confirmation.
	let fake = fake_podman::start_replying(move |method, target| {
		if method == "PUT" && target.contains("/archive?") {
			FakeReply::Body(200, String::new())
		} else {
			FakeReply::Body(404, r#"{"message":"not found"}"#.into())
		}
	});

	let result = upload(&fake, &src, "hello.txt", None).await;
	result.expect("a clean PUT is reported as success");

	let requests = fake.requests.lock().unwrap();
	let put_req = requests
		.iter()
		.find(|r| r.starts_with("PUT ") && r.contains("/archive?"))
		.expect("a /archive PUT was issued");
	let query = put_req
		.split_once('?')
		.expect("the archive PUT carries a query string");
	assert!(
		query.1.contains("copyUIDGID=false"),
		"libpod archive PUT must read `copyUIDGID=false`: {put_req:?}"
	);
	assert!(
		!query.1.split('&').any(|pair| pair == "copyUIDGID=true"),
		"the libpod default of `copyUIDGID=true` must not be sent as-is: {put_req:?}"
	);
}
