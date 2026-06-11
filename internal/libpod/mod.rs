//! Podman libpod REST API client and types.
//!
//! Talks to Podman's native libpod API at `/libpod/...` instead of the
//! Docker-compatibility layer. Eliminates the Docker compat intermediary,
//! gives access to Podman-native error messages, and removes the bollard
//! dependency.

pub mod client;
pub mod error;
pub mod types;

pub use client::Client;
pub(crate) use client::urlencoded;
pub use error::PodmanError;
pub use types::stream::{LogOutput, parse_json_lines, parse_multiplexed, parse_raw};

