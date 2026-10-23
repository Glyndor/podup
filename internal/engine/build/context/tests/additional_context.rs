use super::*;

#[test]
fn additional_context_prefix_mapping() {
	let base = Path::new("/proj");
	assert_eq!(
		map_additional_context(base, "docker-image://alpine"),
		"image:alpine"
	);
	assert_eq!(
		map_additional_context(base, "https://example.org/ctx.tar"),
		"url:https://example.org/ctx.tar"
	);
	assert_eq!(
		map_additional_context(base, "git://example.org/r.git"),
		"url:git://example.org/r.git"
	);
	// Local paths are resolved against base_dir using the host's native path
	// separator, so compare against a `join`ed path rather than a literal.
	let local = map_additional_context(base, "sub/dir");
	assert!(local.starts_with("localpath:"));
	assert!(local.ends_with(&base.join("sub/dir").display().to_string()));
}
