//! Podman socket connection helpers.

// libc FFI (getuid) is needed here; the block carries a soundness comment.
#![allow(unsafe_code)]

use crate::error::{ComposeError, Result};
use crate::libpod::Client;
#[cfg(any(not(windows), test))]
use std::path::Path;

#[cfg(any(not(windows), test))]
const ROOT_SOCKET: &str = "/run/podman/podman.sock";

/// Named pipe `podman machine` exposes on Windows for its default machine.
#[cfg(windows)]
const DEFAULT_PIPE: &str = "//./pipe/podman-machine-default";

/// Connect to Podman's libpod REST API.
///
/// Priority:
/// 1. `socket_path` if provided.
/// 2. The first existing platform default: on Linux the rootful or
///    per-user runtime socket, on macOS the host-side socket exposed by
///    `podman machine`, on Windows the `podman machine` named pipe.
/// 3. The conventional path for this platform, so a failed connection
///    reports the location podup expected.
///
/// Nothing is sent: the socket is opened by the first request. The command
/// line goes through [`connect_checked`], which also confirms the API version.
pub fn connect(socket_path: Option<&str>) -> Result<Client> {
	connect_with_pool_size(socket_path, Client::DEFAULT_POOL_SIZE)
}

/// [`connect`] followed by one `GET /libpod/_ping`, so a Podman below the
/// floor podup supports is refused with [`PodmanError::IncompatibleApiVersion`]
/// before the command sends anything else.
///
/// Every command that talks to Podman connects through here, once, so the
/// check costs one request per command. Until 5.10.2 the check existed
/// (`Client::ping`) and nothing called it: on a host with Podman 4.9 the
/// command failed on its first real request with whatever that request
/// returned, and the message written for the unsupported case never
/// appeared (#1924).
///
/// `pool_size` is the HTTP/1.1 connection-pool cap, floored at 1 by
/// [`Client::with_pool_size`]. The pool is keyed by socket path, so the cap
/// controls the number of concurrent connections a single [`Client`] keeps
/// open to the socket.
///
/// [`PodmanError::IncompatibleApiVersion`]: crate::libpod::PodmanError::IncompatibleApiVersion
pub async fn connect_checked(socket_path: Option<&str>, pool_size: usize) -> Result<Client> {
	let client = connect_with_pool_size(socket_path, pool_size)?;
	client.ping().await?;
	Ok(client)
}

/// As [`connect`], with a caller-chosen HTTP/1.1 connection-pool size. The
/// pool is keyed by socket path, so the cap controls the number of
/// concurrent connections a single [`Client`] will keep open to the socket.
/// `pool_size` is floored at 1 by [`Client::with_pool_size`].
///
/// Crate-private so the command line cannot pick a pool size without the
/// version check that [`connect_checked`] adds.
pub(crate) fn connect_with_pool_size(
	socket_path: Option<&str>,
	pool_size: usize,
) -> Result<Client> {
	let default_path = default_socket_path();
	let raw = socket_path.unwrap_or(&default_path);
	if let Some(scheme) = remote_scheme(raw) {
		return Err(ComposeError::Unsupported(format!(
			"remote Podman over `{scheme}` is not supported; podup talks to a local \
			 rootless socket. Point PODMAN_SOCKET/--socket at a unix:// socket path \
			 (or an npipe:// pipe on Windows)."
		)));
	}
	let path = raw
		.strip_prefix("unix://")
		.or_else(|| raw.strip_prefix("npipe://"))
		.unwrap_or(raw);
	Ok(Client::with_pool_size(path, pool_size))
}

/// Detect a non-local socket scheme (`tcp://`, `ssh://`, `http(s)://`, `fd://`).
/// `unix://`/`npipe://` and plain paths are local and return `None`.
fn remote_scheme(raw: &str) -> Option<&'static str> {
	const REMOTE: [&str; 5] = ["tcp://", "ssh://", "http://", "https://", "fd://"];
	REMOTE.into_iter().find(|s| raw.starts_with(s))
}

/// Read the Podman socket from the environment (`PODMAN_SOCKET`, then
/// `DOCKER_HOST` as a Docker-compatible fallback) and connect. A `unix://` /
/// `npipe://` scheme is stripped by [`connect`]; a remote scheme is rejected
/// there with a clear error.
pub fn connect_from_env() -> Result<Client> {
	connect_from_env_with_pool_size(Client::DEFAULT_POOL_SIZE)
}

/// As [`connect_from_env`], with a caller-chosen HTTP/1.1 connection-pool
/// size (see [`connect_checked`] for what the cap controls).
pub fn connect_from_env_with_pool_size(pool_size: usize) -> Result<Client> {
	let socket = std::env::var("PODMAN_SOCKET")
		.or_else(|_| std::env::var("DOCKER_HOST"))
		.ok();

	connect_with_pool_size(socket.as_deref(), pool_size)
}

#[cfg(not(windows))]
pub(crate) fn default_socket_path() -> String {
	let candidates = candidate_socket_paths();
	first_existing(&candidates)
		.or_else(machine_socket_path)
		.or_else(|| candidates.into_iter().next())
		.unwrap_or_else(|| ROOT_SOCKET.to_string())
}

/// Windows: named pipes are not probeable through `Path::exists`, so ask
/// `podman machine inspect` and fall back to the default machine's pipe.
#[cfg(windows)]
pub(crate) fn default_socket_path() -> String {
	machine_socket_path().unwrap_or_else(|| DEFAULT_PIPE.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn candidate_socket_paths() -> Vec<String> {
	// SAFETY: getuid takes no arguments, touches no memory and cannot fail.
	let uid = unsafe { libc::getuid() };
	let xdg = std::env::var("XDG_RUNTIME_DIR").ok();
	let trusted = xdg.as_deref().filter(|dir| trusted_runtime_dir(dir, uid));
	if xdg.as_deref().is_some_and(|d| !d.is_empty()) && trusted.is_none() {
		tracing::warn!(
			"XDG_RUNTIME_DIR is not an absolute directory owned by this user that neither its group nor \
			 others can write; not looking for the Podman socket there"
		);
	}
	runtime_candidates(uid, trusted)
}

/// Whether `dir` may name the place the implicit Podman socket is looked up in:
/// an absolute path to a real directory (not a link) that this user owns and
/// that neither the group nor others can write to. A directory somebody else
/// can write to lets them put a socket there that answers the ping like Podman
/// and then receives every request, so the implicit lookup ignores it. An
/// explicit `--socket` or `PODMAN_SOCKET` is the operator's own choice and is
/// not checked.
#[cfg(all(unix, not(target_os = "macos")))]
fn trusted_runtime_dir(dir: &str, uid: u32) -> bool {
	use std::os::unix::fs::MetadataExt;
	let path = Path::new(dir);
	if !path.is_absolute() {
		return false;
	}
	match std::fs::symlink_metadata(path) {
		Ok(meta) => meta.is_dir() && meta.uid() == uid && meta.mode() & 0o022 == 0,
		Err(_) => false,
	}
}

#[cfg(target_os = "macos")]
fn candidate_socket_paths() -> Vec<String> {
	match std::env::var("HOME") {
		Ok(home) => machine_candidates(&home),
		Err(_) => vec![ROOT_SOCKET.to_string()],
	}
}

/// Socket candidates for Linux and other unix hosts: the rootful socket
/// for uid 0, otherwise the user's runtime directory (preferring
/// `XDG_RUNTIME_DIR` when set).
#[cfg(any(all(unix, not(target_os = "macos")), test))]
fn runtime_candidates(uid: u32, xdg_runtime_dir: Option<&str>) -> Vec<String> {
	if uid == 0 {
		return vec![ROOT_SOCKET.to_string()];
	}
	let mut candidates = Vec::new();
	if let Some(dir) = xdg_runtime_dir {
		if !dir.is_empty() {
			candidates.push(format!("{dir}/podman/podman.sock"));
		}
	}
	let run_user = format!("/run/user/{uid}/podman/podman.sock");
	if !candidates.contains(&run_user) {
		candidates.push(run_user);
	}
	candidates
}

/// Socket candidates on macOS: the host-side sockets `podman machine`
/// creates, newest layout first. Podman 5 names the per-provider directory
/// after the active machine provider (`applehv` by default, `vz` for the
/// Virtualization.framework backend), so those are tried before the older
/// `qemu`/default layouts.
#[cfg(any(target_os = "macos", test))]
fn machine_candidates(home: &str) -> Vec<String> {
	let machine_dir = format!("{home}/.local/share/containers/podman/machine");
	vec![
		format!("{machine_dir}/podman.sock"),
		format!("{machine_dir}/applehv/podman.sock"),
		format!("{machine_dir}/vz/podman.sock"),
		format!("{machine_dir}/qemu/podman.sock"),
		format!("{machine_dir}/podman-machine-default/podman.sock"),
	]
}

/// Ask `podman machine inspect` for the host-side socket path. Only used
/// on macOS, where the VM provider decides where the API socket lives.
#[cfg(target_os = "macos")]
fn machine_socket_path() -> Option<String> {
	let output = std::process::Command::new("podman")
		.args([
			"machine",
			"inspect",
			"--format",
			"{{ .ConnectionInfo.PodmanSocket.Path }}",
		])
		.output()
		.ok()?;
	if !output.status.success() {
		return None;
	}
	let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
	(!path.is_empty() && Path::new(&path).exists()).then_some(path)
}

/// Ask `podman machine inspect` for the named pipe of the default machine.
/// The pipe path is reported by the machine config; `Path::exists` cannot
/// probe named pipes, so the value is used as-is.
#[cfg(windows)]
fn machine_socket_path() -> Option<String> {
	let output = std::process::Command::new("podman")
		.args([
			"machine",
			"inspect",
			"--format",
			"{{ .ConnectionInfo.PodmanPipe.Path }}",
		])
		.output()
		.ok()?;
	if !output.status.success() {
		return None;
	}
	let path = String::from_utf8(output.stdout).ok()?.trim().to_string();
	(!path.is_empty() && path != "<nil>").then_some(path)
}

#[cfg(all(unix, not(target_os = "macos")))]
fn machine_socket_path() -> Option<String> {
	None
}

#[cfg(any(not(windows), test))]
fn first_existing(candidates: &[String]) -> Option<String> {
	candidates.iter().find(|p| Path::new(p).exists()).cloned()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
