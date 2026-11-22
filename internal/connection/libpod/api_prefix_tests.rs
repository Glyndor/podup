//! The route prefix and the version floor name the same release.

use super::client::MIN_PODMAN_VERSION;
use super::API_PREFIX;

#[test]
fn api_prefix_names_the_minimum_podman_version() {
	let (major, minor, patch) = MIN_PODMAN_VERSION;
	assert_eq!(API_PREFIX, format!("/v{major}.{minor}.{patch}/libpod"));
}
