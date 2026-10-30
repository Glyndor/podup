use crate::engine::lifecycle::container_rm_path;

#[test]
fn rm_path_omits_volume_flag_by_default() {
	// A plain `down` (or scale-down) must not drop volumes.
	let path = container_rm_path("proj-web-1", false, true);
	assert!(path.ends_with("/proj-web-1?force=true"), "got: {path}");
	assert!(!path.contains("volumes="), "got: {path}");
}

#[test]
fn rm_path_requests_anonymous_volume_removal() {
	// `down -v` must pass `volumes=true` so podman reclaims the container's
	// anonymous (image VOLUME / short-form) volumes. The libpod
	// `/containers/{id}` delete endpoint reads `volumes=` for that; the
	// Docker compat handler used `v=`, which the libpod handler ignores
	// (so a `down -v` against libpod without `volumes=` reclaims nothing).
	let path = container_rm_path("proj-web-1", true, true);
	assert!(path.contains("force=true"), "got: {path}");
	assert!(path.contains("&volumes=true"), "got: {path}");
}

#[test]
fn rm_path_url_encodes_container_name() {
	// Names are URL-encoded so a slash in a container name cannot alter the
	// request path.
	let path = container_rm_path("weird/name", true, true);
	assert!(!path.contains("weird/name"), "got: {path}");
	assert!(path.contains("weird%2Fname"), "got: {path}");
}

#[test]
fn rm_path_force_false_passes_force_false() {
	// `podup rm` (no `-f`) must not force a running container down; the libpod
	// delete endpoint honours `force=false` by rejecting a still-running
	// container with a 409. The shared helper passes the caller's flag
	// through.
	let path = container_rm_path("proj-web-1", false, false);
	assert!(path.contains("force=false"), "got: {path}");
	assert!(!path.contains("force=true"), "got: {path}");
}
