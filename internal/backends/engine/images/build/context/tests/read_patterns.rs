use super::*;

#[test]
fn dockerignore_accepts_bom_and_only_column_one_comments() {
	let dir = tempdir().unwrap();
	fs::write(
		dir.path().join(".dockerignore"),
		"\u{feff}/.env\n# comment\n  #literal\n",
	)
	.unwrap();
	let (_, patterns) = read_patterns(dir.path());
	assert_eq!(patterns, vec!["/.env", "#literal"]);
}

// read_patterns ----------------------------------------------------------

#[test]
fn dockerignore_parsed_correctly() {
	let dir = tempdir().unwrap();
	fs::write(
		dir.path().join(".dockerignore"),
		b"# comment\n\ntarget/\n*.log\n",
	)
	.unwrap();
	let (name, patterns) = read_patterns(dir.path());
	assert_eq!(name, ".dockerignore");
	assert_eq!(patterns, vec!["target/", "*.log"]);
}

#[test]
fn containerignore_is_read_when_it_is_the_only_one() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join(".containerignore"), b"secrets/\n*.key\n").unwrap();
	let (name, patterns) = read_patterns(dir.path());
	assert_eq!(name, ".containerignore");
	assert_eq!(patterns, vec!["secrets/", "*.key"]);
}

/// podman-build(1): "if both are in the context directory, podman build only
/// uses `.containerignore`". Applying both client-side is what produced an image
/// missing content that `podman build` includes.
#[test]
fn containerignore_wins_and_dockerignore_is_not_merged() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join(".containerignore"), b"a.txt\n").unwrap();
	fs::write(dir.path().join(".dockerignore"), b"b.txt\n").unwrap();
	let (name, patterns) = read_patterns(dir.path());
	assert_eq!(name, ".containerignore");
	assert_eq!(
		patterns,
		vec!["a.txt"],
		"the two files must never be unioned"
	);
}

/// With neither present the name still matters: it is what a synthesized ignore
/// file is written under, and podman prefers `.containerignore`, so choosing it
/// means our exclusions cannot be shadowed by a `.dockerignore`.
#[test]
fn no_ignore_file_defaults_to_containerignore_with_no_patterns() {
	let dir = tempdir().unwrap();
	let (name, patterns) = read_patterns(dir.path());
	assert_eq!(name, ".containerignore");
	assert!(patterns.is_empty());
}
