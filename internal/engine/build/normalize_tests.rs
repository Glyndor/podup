//! Pure unit tests for the build-path image-name normalisation
//! helpers.
//!
//! The pure normalisation rule lives in
//! [`crate::libpod::normalize::normalize_docker_reference`] and is
//! tested there. This file pins the wire-shaped half: picking the
//! matching `RepoTags` entry from a local-image inspect response
//! when the input already resolves to a local image. The match
//! function is the one decision a unit test can drive without a
//! live socket.

use crate::engine::build::normalize::match_repo_tag;

#[test]
fn matches_when_input_tag_suffix_is_present() {
	let tags = vec![
		"docker.io/library/alpine:3.20".to_string(),
		"localhost/proj/app:v1".to_string(),
	];
	assert_eq!(
		match_repo_tag(&tags, "proj/app:v1"),
		Some("localhost/proj/app:v1"),
		"the input `proj/app:v1` must pick the RepoTag that ends with `:v1` (the \
		 daemon already resolved the name; the build must keep the same canonical form)"
	);
}

#[test]
fn matches_when_input_has_no_tag() {
	let tags = vec!["docker.io/library/alpine:3.20".to_string()];
	// A bare-name input cannot match a tagged RepoTag (no `:` in
	// the input -> any entry with `:` is tagged, so we look for an
	// entry that is itself bare). With no bare entry, fall back to
	// the first RepoTag so the wire path still has a canonical name.
	assert_eq!(
		match_repo_tag(&tags, "alpine"),
		Some("docker.io/library/alpine:3.20"),
		"a bare-name input falls back to the first RepoTag when no untagged entry exists"
	);
}

#[test]
fn empty_repo_tags_yields_none() {
	let tags: Vec<String> = Vec::new();
	assert!(
		match_repo_tag(&tags, "anything").is_none(),
		"an empty RepoTags list must produce no match; the caller then falls back to \
		 the pure normalisation rule"
	);
}

#[test]
fn picks_first_matching_tag_when_multiple_match() {
	// Two RepoTags both end with `:v1`. The first one wins so the
	// result is deterministic on the same local storage state.
	let tags = vec![
		"docker.io/proj/app:v1".to_string(),
		"localhost/proj/app:v1".to_string(),
	];
	assert_eq!(
		match_repo_tag(&tags, "proj/app:v1"),
		Some("docker.io/proj/app:v1"),
		"the first matching RepoTag wins; the wire-shape contract is deterministic"
	);
}
