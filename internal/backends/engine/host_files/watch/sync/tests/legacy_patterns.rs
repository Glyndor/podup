use super::*;

#[test]
fn ignored_exact_file() {
	assert!(legacy_project_relative_ignored(
		"Makefile",
		&pats(&["Makefile"])
	));
}

#[test]
fn ignored_not_prefix_match() {
	assert!(!legacy_project_relative_ignored(
		"Makefile.local",
		&pats(&["Makefile"])
	));
}

#[test]
fn ignored_dir_prefix() {
	assert!(legacy_project_relative_ignored(
		"node_modules/foo.js",
		&pats(&["node_modules/"])
	));
}

#[test]
fn ignored_dir_prefix_no_partial() {
	assert!(!legacy_project_relative_ignored(
		"nonode_modules/foo",
		&pats(&["node_modules/"])
	));
}

#[test]
fn ignored_path_with_slash() {
	assert!(legacy_project_relative_ignored(
		"vendor/lib.rs",
		&pats(&["vendor"])
	));
}

#[test]
fn ignored_empty_patterns() {
	assert!(!legacy_project_relative_ignored("anything.rs", &[]));
}

#[test]
fn ignored_no_match() {
	assert!(!legacy_project_relative_ignored(
		"src/main.rs",
		&pats(&["target/", "*.log"])
	));
}

// legacy_project_relative_included ----------------------------------------------------------

#[test]
fn included_glob_extension() {
	assert!(legacy_project_relative_included(
		"src/main.rs",
		&pats(&["*.rs"])
	));
}

#[test]
fn included_glob_no_match() {
	assert!(!legacy_project_relative_included(
		"src/main.go",
		&pats(&["*.rs"])
	));
}

#[test]
fn included_dir_prefix() {
	assert!(legacy_project_relative_included(
		"src/main.rs",
		&pats(&["src/"])
	));
}

#[test]
fn included_dir_prefix_no_match() {
	assert!(!legacy_project_relative_included(
		"test/main.rs",
		&pats(&["src/"])
	));
}

#[test]
fn included_exact_match() {
	assert!(legacy_project_relative_included(
		"Makefile",
		&pats(&["Makefile"])
	));
}

#[test]
fn included_path_segment_suffix() {
	assert!(legacy_project_relative_included(
		"src/lib.rs",
		&pats(&["lib.rs"])
	));
}

#[test]
fn included_empty_patterns() {
	assert!(!legacy_project_relative_included("anything", &[]));
}

// build_sync_tar -------------------------------------------------------
