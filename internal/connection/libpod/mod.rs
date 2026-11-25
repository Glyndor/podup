//! Podman libpod REST API client and types.
//!
//! Talks to Podman's native libpod API at `/libpod/...` instead of the
//! Docker-compatibility layer. Eliminates the Docker compat intermediary,
//! gives access to Podman-native error messages, and removes the bollard
//! dependency.

pub mod client;
pub mod error;
pub(crate) mod normalize;
pub mod types;
pub(crate) mod validate;

/// Path prefix for every libpod REST route.
///
/// Podman's libpod routes are version-namespaced: an unversioned path such as
/// `/libpod/containers/json` is not guaranteed to resolve, so every request
/// must carry the API version. The prefix names the lowest Podman release podup
/// accepts ([`client::MIN_PODMAN_VERSION`]); a test keeps the two in step. The
/// version-independent `/libpod/_ping` endpoint is the sole exception and
/// deliberately omits this prefix.
pub(crate) const API_PREFIX: &str = "/v6.2.0/libpod";

#[cfg(test)]
mod api_prefix_tests;

pub use client::Client;
pub use client::DrivenBody;
pub(crate) use client::{image_urlencoded, is_valid_object_name, urlencoded};
pub use error::PodmanError;
pub use types::stream::{parse_json_lines, parse_multiplexed, parse_raw, LogOutput};
