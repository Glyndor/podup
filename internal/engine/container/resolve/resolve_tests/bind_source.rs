use super::*;

#[test]
#[cfg(unix)]
fn bind_source_resolution() {
	use std::path::Path;
	let base = Path::new("/srv/app");
	assert_eq!(resolve_bind_source("/abs/path", base), "/abs/path");
	assert_eq!(resolve_bind_source("./data", base), "/srv/app/./data");
	assert_eq!(resolve_bind_source("data", base), "/srv/app/data");
	// Mutating HOME via the process env races other tests under the parallel
	// runner; temp_env::with_var sets and restores it atomically.
	temp_env::with_var("HOME", Some("/home/u"), || {
		assert_eq!(resolve_bind_source("~/x", base), "/home/u/x");
		assert_eq!(resolve_bind_source("~", base), "/home/u");
	});
	// An empty source is returned verbatim (no base-dir join).
	assert_eq!(resolve_bind_source("", base), "");
}

#[test]
#[cfg(unix)]
fn bind_source_tilde_without_home_stays_literal() {
	use std::path::Path;
	let base = Path::new("/srv/app");
	// With no home directory resolvable, a `~`-prefixed path keeps its literal
	// form, then (being relative) is anchored to the base dir.
	temp_env::with_vars(
		[("HOME", None::<&str>), ("USERPROFILE", None::<&str>)],
		|| {
			assert_eq!(resolve_bind_source("~/x", base), "/srv/app/~/x");
			assert_eq!(resolve_bind_source("~", base), "/srv/app/~");
		},
	);
}
