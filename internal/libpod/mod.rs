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
pub use types::stream::{LogOutput, parse_json_lines, parse_multiplexed};

use crate::error::Result;

/// Connect to Podman via its Unix socket or Windows named pipe.
///
/// Priority:
/// 1. `socket_path` if provided.
/// 2. Platform default — Linux rootful or per-user runtime socket, macOS
///    `podman machine` socket, Windows `podman machine` named pipe.
pub fn connect(socket_path: Option<&str>) -> Result<Client> {
	let default = crate::podman::default_socket_path();
	let path = socket_path.unwrap_or(default.as_str());
	Ok(Client::new(path))
}

/// Connect using `PODMAN_SOCKET` or `DOCKER_HOST` environment variables,
/// stripping the `unix://` or `npipe://` scheme prefix if present.
pub fn connect_from_env() -> Result<Client> {
	let socket = std::env::var("PODMAN_SOCKET")
		.or_else(|_| std::env::var("DOCKER_HOST"))
		.ok();

	let path = socket.as_deref().and_then(|s| {
		s.strip_prefix("unix://")
			.or_else(|| s.strip_prefix("npipe://"))
	});

	connect(path)
}
