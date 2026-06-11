//! Podman libpod REST API client and types.
//!
//! Talks to Podman's native libpod API at `/libpod/...` instead of the
//! Docker-compatibility layer. Eliminates the Docker compat intermediary,
//! gives access to Podman-native error messages, and removes the bollard
//! dependency.

pub mod client;
pub mod error;
pub mod types;

pub(crate) use client::urlencoded;
pub use client::Client;
pub use error::PodmanError;
pub use types::stream::{parse_json_lines, parse_multiplexed, parse_raw, LogOutput};

/// Version prefix required for all libpod REST API endpoints.
///
/// Podman registers libpod routes only at versioned paths
/// (`/v{N}.{M}.{P}/libpod/...`). The unversioned `/libpod/...` form returns
/// 404 for every route except `/_ping`. Any version string the server
/// supports resolves to the same handler; `v4.0.0` is the minimum version
/// podup requires.
pub(crate) const LIBPOD: &str = "/v4.0.0/libpod";
