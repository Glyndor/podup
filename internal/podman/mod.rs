//! Podman socket connection helpers.

use crate::error::Result;
use bollard::Docker;
#[cfg(any(not(windows), test))]
use std::path::Path;

#[cfg(any(not(windows), test))]
const ROOT_SOCKET: &str = "/run/podman/podman.sock";

/// Named pipe `podman machine` exposes on Windows for its default machine.
#[cfg(windows)]
const DEFAULT_PIPE: &str = "//./pipe/podman-machine-default";

/// Connect to Podman's Docker-compatible API.
///
/// Priority:
/// 1. `socket_path` if provided.
/// 2. The first existing platform default — on Linux the rootful or
///    per-user runtime socket, on macOS the host-side socket exposed by
///    `podman machine`, on Windows the `podman machine` named pipe.
/// 3. The conventional path for this platform, so a failed connection
///    reports the location podup expected.
pub fn connect(socket_path: Option<&str>) -> Result<Docker> {
	let default_path = default_socket_path();
	let path = socket_path.unwrap_or(&default_path);
	#[cfg(not(windows))]
	let client = Docker::connect_with_unix(path, 120, bollard::API_DEFAULT_VERSION)?;
	#[cfg(windows)]
	let client = Docker::connect_with_named_pipe(path, 120, bollard::API_DEFAULT_VERSION)?;
	Ok(client)
}

/// Strips the `unix://` or `npipe://` scheme prefix before passing the path to [`connect`].
pub fn connect_from_env() -> Result<Docker> {
	let socket = std::env::var("PODMAN_SOCKET")
		.or_else(|_| std::env::var("DOCKER_HOST"))
		.ok();

	let path = socket.as_deref().and_then(|s| {
		s.strip_prefix("unix://")
			.or_else(|| s.strip_prefix("npipe://"))
	});
	connect(path)
}

#[cfg(not(windows))]
fn default_socket_path() -> String {
	let candidates = candidate_socket_paths();
	first_existing(&candidates)
		.or_else(machine_socket_path)
		.or_else(|| candidates.into_iter().next())
		.unwrap_or_else(|| ROOT_SOCKET.to_string())
}

/// Windows: named pipes are not probeable through `Path::exists`, so ask
/// `podman machine inspect` and fall back to the default machine's pipe.
#[cfg(windows)]
fn default_socket_path() -> String {
	machine_socket_path().unwrap_or_else(|| DEFAULT_PIPE.to_string())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn candidate_socket_paths() -> Vec<String> {
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
/// creates, newest layout first.
#[cfg(any(target_os = "macos", test))]
fn machine_candidates(home: &str) -> Vec<String> {
	let machine_dir = format!("{home}/.local/share/containers/podman/machine");
	vec![
		format!("{machine_dir}/podman.sock"),
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
mod tests {
	use super::*;

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
				format!("{machine_dir}/qemu/podman.sock"),
				format!("{machine_dir}/podman-machine-default/podman.sock"),
			]
		);
	}
}
