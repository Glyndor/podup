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
/// 2. The first existing platform default — on Linux the rootful or
///    per-user runtime socket, on macOS the host-side socket exposed by
///    `podman machine`, on Windows the `podman machine` named pipe.
/// 3. The conventional path for this platform, so a failed connection
///    reports the location podup expected.
pub fn connect(socket_path: Option<&str>) -> Result<Client> {
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
	Ok(Client::new(path))
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
	let socket = std::env::var("PODMAN_SOCKET")
		.or_else(|_| std::env::var("DOCKER_HOST"))
		.ok();

	connect(socket.as_deref())
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
	runtime_candidates(uid, std::env::var("XDG_RUNTIME_DIR").ok().as_deref())
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

/// The minimum Podman major version podup's libpod API surface requires.
const MIN_PODMAN_MAJOR: u32 = 5;

/// Whether a Podman version string (e.g. `"5.4.2"`) meets the minimum major
/// version. An unparsable version is treated as unsupported (fail closed).
fn version_meets_minimum(version: &str) -> bool {
	version
		.split('.')
		.next()
		.and_then(|major| major.parse::<u32>().ok())
		.is_some_and(|major| major >= MIN_PODMAN_MAJOR)
}

/// Decide whether a reported version is supported, returning the user-facing
/// error otherwise. Pure so both branches are unit-tested without a socket.
fn check_version(version: &str) -> Result<()> {
	if version_meets_minimum(version) {
		Ok(())
	} else {
		Err(ComposeError::Unsupported(format!(
			"podup requires Podman >= {MIN_PODMAN_MAJOR}.0, but the daemon reports {version}; \
			 upgrade Podman or point --socket at a {MIN_PODMAN_MAJOR}.x daemon"
		)))
	}
}

/// Fail fast with a clear message when the connected daemon is older than the
/// supported Podman major version. podup speaks the versioned libpod API
/// (`/v5.0.0/libpod/...`); a Podman < 5 answers 404 to every call, which would
/// otherwise surface as an opaque "HTTP 404" on the first real operation.
pub async fn ensure_supported_version(client: &Client) -> Result<()> {
	let version = client
		.podman_version()
		.await
		.map_err(ComposeError::Podman)?;
	check_version(&version)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn version_minimum_accepts_5_and_above_rejects_older_and_garbage() {
		for ok in ["5.0.0", "5.4.2", "6.1.0", "10.0.0"] {
			assert!(version_meets_minimum(ok), "{ok} should be supported");
		}
		for bad in ["4.9.4", "3.4.0", "garbage", "", "v5"] {
			assert!(!version_meets_minimum(bad), "{bad} should be unsupported");
		}
	}

	#[test]
	fn check_version_passes_supported_and_rejects_old_with_message() {
		assert!(check_version("5.4.2").is_ok());
		let err = check_version("4.9.4").unwrap_err();
		assert!(matches!(err, ComposeError::Unsupported(_)));
		let msg = err.to_string();
		assert!(msg.contains("4.9.4") && msg.contains("requires Podman"));
	}

	#[test]
	fn first_existing_picks_first_match() {
		let dir = tempfile::tempdir().unwrap();
		let hit = dir.path().join("podman.sock");
		std::fs::write(&hit, b"").unwrap();
		let candidates = vec![
			dir.path().join("missing.sock").display().to_string(),
			hit.display().to_string(),
			dir.path().join("later.sock").display().to_string(),
		];
		assert_eq!(first_existing(&candidates), Some(hit.display().to_string()));
	}

	#[test]
	fn first_existing_none_when_no_candidate_exists() {
		let candidates = vec!["/nonexistent/podup-test/podman.sock".to_string()];
		assert_eq!(first_existing(&candidates), None);
	}

	#[test]
	fn runtime_candidates_root_uses_system_socket() {
		let candidates = runtime_candidates(0, Some("/run/user/0"));
		assert_eq!(candidates, vec![ROOT_SOCKET.to_string()]);
	}

	#[test]
	fn runtime_candidates_prefers_xdg_runtime_dir() {
		let candidates = runtime_candidates(1000, Some("/custom/runtime"));
		assert_eq!(
			candidates,
			vec![
				"/custom/runtime/podman/podman.sock".to_string(),
				"/run/user/1000/podman/podman.sock".to_string(),
			]
		);
	}

	#[test]
	fn runtime_candidates_dedupes_default_runtime_dir() {
		let candidates = runtime_candidates(1000, Some("/run/user/1000"));
		assert_eq!(
			candidates,
			vec!["/run/user/1000/podman/podman.sock".to_string()]
		);
	}

	#[test]
	fn runtime_candidates_ignores_empty_runtime_dir() {
		let candidates = runtime_candidates(1000, Some(""));
		assert_eq!(
			candidates,
			vec!["/run/user/1000/podman/podman.sock".to_string()]
		);
	}

	#[test]
	fn machine_candidates_cover_known_layouts() {
		let machine_dir = "/Users/dev/.local/share/containers/podman/machine";
		assert_eq!(
			machine_candidates("/Users/dev"),
			vec![
				format!("{machine_dir}/podman.sock"),
				format!("{machine_dir}/applehv/podman.sock"),
				format!("{machine_dir}/vz/podman.sock"),
				format!("{machine_dir}/qemu/podman.sock"),
				format!("{machine_dir}/podman-machine-default/podman.sock"),
			]
		);
	}

	#[test]
	fn connect_strips_unix_scheme() {
		let c = connect(Some("unix:///run/user/1000/podman/podman.sock")).unwrap();
		drop(c);
	}

	#[test]
	fn connect_strips_npipe_scheme() {
		let c = connect(Some("npipe:////./pipe/podman")).unwrap();
		drop(c);
	}

	#[test]
	fn connect_passes_plain_path_unchanged() {
		let c = connect(Some("/run/user/1000/podman/podman.sock")).unwrap();
		drop(c);
	}

	#[test]
	fn connect_rejects_remote_schemes() {
		for raw in [
			"tcp://127.0.0.1:2375",
			"ssh://user@host/run/podman.sock",
			"http://localhost:8080",
			"https://localhost:8080",
			"fd://3",
		] {
			assert!(
				matches!(connect(Some(raw)), Err(ComposeError::Unsupported(_))),
				"{raw} should be rejected as unsupported"
			);
		}
	}

	#[test]
	fn remote_scheme_ignores_local_sockets() {
		assert!(remote_scheme("/run/podman.sock").is_none());
		assert!(remote_scheme("unix:///run/podman.sock").is_none());
		assert!(remote_scheme("npipe:////./pipe/podman").is_none());
	}
}
