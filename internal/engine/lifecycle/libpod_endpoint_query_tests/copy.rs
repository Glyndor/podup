/// `PUT /containers/{}/archive` must carry `copyUIDGID=false`. The
/// libpod handler defaults `copyUIDGID` to true, which makes a host
/// file copied into a container take the container's runtime UID/GID
/// (i.e. `0:0`). The Docker compat handler defaulted it to false,
/// which preserved the host UID/GID on the destination file (as measured with podup 5.10.0 on
/// 2026-09-24: `1000:1000`). The compensation pins the
/// docker-compat default.
///
/// The wire-level test for the compensation lives next to the rest of
/// the `cp` upload tests in `engine::copy::upload_tests::upload_carries_copy_uid_gid_false`,
/// which drives the same fake podman through the streaming packer
/// `cp_to_container` uses (the packer is private to the `copy`
/// module). What is pinned here is the URL helper the production path
/// is built on, so a regression that dropped the parameter is caught
/// even when the streaming packer cannot be reached.
#[test]
fn archive_put_path_includes_copy_uid_gid_false() {
	let path = crate::engine::copy::upload::archive_put_path("proj-web-1", "/tmp");
	assert!(
		path.contains("copyUIDGID=false"),
		"libpod archive PUT must carry `copyUIDGID=false`: {path}"
	);
	assert!(
		!path.split('&').any(|pair| pair == "copyUIDGID=true"),
		"the libpod default of `copyUIDGID=true` must not be sent as-is: {path}"
	);
}
