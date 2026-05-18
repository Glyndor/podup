//! Podman socket connection helpers.

use crate::error::Result;
use bollard::Docker;

const DEFAULT_PODMAN_SOCKET: &str = "/run/podman/podman.sock";

/// Connect to Podman's Docker-compatible socket.
///
/// Priority:
/// 1. `socket_path` if provided.
/// 2. `/run/podman/podman.sock` when running as root.
/// 3. `/run/user/<uid>/podman/podman.sock` for non-root users.
pub fn connect(socket_path: Option<&str>) -> Result<Docker> {
    let default_path = default_socket_path();
    let path = socket_path.unwrap_or(&default_path);
    let client = Docker::connect_with_unix(path, 120, bollard::API_DEFAULT_VERSION)?;
    Ok(client)
}

fn default_socket_path() -> String {
    let uid = unsafe { libc::getuid() };
    if uid == 0 {
        DEFAULT_PODMAN_SOCKET.to_string()
    } else {
        format!("/run/user/{uid}/podman/podman.sock")
    }
}

/// Connect using `PODMAN_SOCKET` or `DOCKER_HOST` environment variables.
pub fn connect_from_env() -> Result<Docker> {
    let socket = std::env::var("PODMAN_SOCKET")
        .or_else(|_| std::env::var("DOCKER_HOST"))
        .ok();

    let path = socket.as_deref().and_then(|s| s.strip_prefix("unix://"));
    connect(path)
}
