use super::{abs_context, qualify_local_image_tag};
use std::path::Path;

#[test]
fn abs_context_makes_relative_build_contexts_absolute() {
	let base = Path::new("/srv/app");
	// `.` is the compose file's own directory.
	assert_eq!(abs_context(base, "."), "/srv/app");
	// A `./`-prefixed or bare relative path joins under the base, kept clean.
	assert_eq!(abs_context(base, "./src"), "/srv/app/src");
	assert_eq!(abs_context(base, "src"), "/srv/app/src");
	// A parent traversal is preserved (systemd/podman resolve it).
	assert_eq!(abs_context(base, "../shared"), "/srv/app/../shared");
	// An already-absolute context is passed through untouched.
	assert_eq!(abs_context(base, "/opt/build"), "/opt/build");
}

/// Pin every branch of the qualifier so the prebuilt container's `Image=`
/// always names the image the build step stored.
///
/// Each input/output pair was checked against `podman build -t <input>`
/// on Podman 5.x: `podman images --format '{{.Repository}}:{{.Tag}}'`
/// after the build shows `localhost/<input>:latest`, and
/// `podman run --pull never <output>` resolves to that exact entry.
#[test]
fn qualify_local_image_tag_pins_short_names_to_localhost() {
	// The project/service fallback tag the build unit writes when the
	// service declares no `image:`. Short, no registry, must be qualified.
	assert_eq!(qualify_local_image_tag("proj-web"), "localhost/proj-web");
	// The measured regression input from the review: a bare image: alpine
	// got `docker.io/library/alpine:latest` at run-time; the build wrote
	// `localhost/alpine:latest`. Qualifier closes the gap.
	assert_eq!(qualify_local_image_tag("alpine"), "localhost/alpine");
	// A user/repo path on Docker Hub is a short name too: there is no
	// `.` or `:` in the registry part, so `localhost/` must be added
	// (otherwise it would still resolve via the registry search list).
	assert_eq!(
		qualify_local_image_tag("user/app:1"),
		"localhost/user/app:1"
	);
	// A real registry (a `.` in the registry part) is fully qualified and
	// must NOT be re-prefixed: the build step already stored the tag under
	// that registry.
	assert_eq!(
		qualify_local_image_tag("reg.example/app:1"),
		"reg.example/app:1"
	);
	// A registry with a port (`:` in the registry part) is also fully
	// qualified; do not re-prefix.
	assert_eq!(
		qualify_local_image_tag("localhost:5000/app"),
		"localhost:5000/app"
	);
	// Bare `localhost/<app>` is the explicit form the build wrote for a
	// short tag; leave it alone.
	assert_eq!(qualify_local_image_tag("localhost/app"), "localhost/app");
}
