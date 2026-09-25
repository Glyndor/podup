//! Pure unit tests for [`super::normalize_docker_reference`].
//!
//! Five inputs match the five shapes the docker compat build handler
//! distinguished through `NormalizeToDockerHub`: a library image
//! with an explicit tag, an org-scoped image with no tag, a name
//! whose first component is registry-qualified (left alone), a
//! digest (also left alone), and a tag-less library name. The fix
//! is wire-shaped; the function lives in production code so the test
//! pins the exact strings the helper produces.

use crate::libpod::normalize::normalize_docker_reference;

#[test]
fn library_with_explicit_tag_expands_to_docker_io_canonical() {
	assert_eq!(
		normalize_docker_reference("app:latest"),
		"docker.io/library/app:latest",
		"`app:latest` must become the docker.io library canonical form (matching the \
		 docker compat handler's `NormalizeToDockerHub` output)"
	);
}

#[test]
fn org_scoped_name_without_tag_expands_to_docker_io_canonical() {
	assert_eq!(
		normalize_docker_reference("org/app"),
		"docker.io/org/app",
		"`org/app` must become the docker.io org canonical form, with no `:latest` \
		 added (matching `reference.ParseNormalizedNamed`'s output)"
	);
}

#[test]
fn registry_qualified_name_passes_through_unchanged() {
	for input in [
		"localhost/foo",
		"localhost/foo:latest",
		"host:5000/foo",
		"host:5000/foo:v1",
		"quay.io/foo",
		"quay.io/foo:latest",
	] {
		assert_eq!(
			normalize_docker_reference(input),
			input,
			"a name whose first component contains `.` or `:` or is `localhost` must \
			 pass through unchanged; `{input}` round-tripped correctly when the helper \
			 returned `{input}` (got a different value)"
		);
	}
}

#[test]
fn digest_passes_through_unchanged() {
	let digest = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
	assert_eq!(
		normalize_docker_reference(digest),
		digest,
		"a `sha256:...` digest must pass through unchanged; the docker compat handler \
		 left it alone because it cannot parse a digest as a named reference"
	);
}

#[test]
fn tag_less_library_name_expands_to_docker_io_library_with_latest() {
	// The pure helper emits the no-tag form (`docker.io/library/app`)
	// because `reference.ParseNormalizedNamed` does the same: a tag is
	// only added when one is present in the input. The `apply_extra_tags`
	// call site defaults a missing tag to `latest` after splitting.
	assert_eq!(
		normalize_docker_reference("app"),
		"docker.io/library/app",
		"a tag-less library name must expand to the docker.io canonical form without \
		 a tag (`reference.ParseNormalizedNamed` does the same)"
	);
}
