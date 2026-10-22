//! Unit tests for `socket_error`: the message an operator sees when the Podman
//! socket cannot be opened has to name the path, tell the two failures apart,
/// and point at the fix that applies to each (#1146).
use super::socket_error;

/// The operator's report (#1146): `podman socket connection error: No such
/// file or directory (os error 2)`: no path, no way to tell "it is not there"
/// from "I cannot open it", nothing to act on. Everything needed was already in
/// hand one call earlier.
#[cfg(unix)]
#[test]
fn a_missing_socket_names_the_path_and_the_fix() {
	let e = socket_error(
		"/run/user/1000/podman/podman.sock",
		std::io::Error::from(std::io::ErrorKind::NotFound),
	);
	let msg = e.to_string();
	assert!(msg.contains("/run/user/1000/podman/podman.sock"), "{msg}");
	assert!(
		msg.contains("systemctl --user enable --now podman.socket"),
		"{msg}"
	);
	assert!(msg.contains("XDG_RUNTIME_DIR"), "{msg}");
}

/// A socket that exists but cannot be opened is a different problem with a
/// different fix, and the old message could not tell them apart.
#[cfg(unix)]
#[test]
fn a_denied_socket_says_so_rather_than_suggesting_enabling_it() {
	let e = socket_error(
		"/tmp/s.sock",
		std::io::Error::from(std::io::ErrorKind::PermissionDenied),
	);
	let msg = e.to_string();
	assert!(msg.contains("/tmp/s.sock"), "{msg}");
	assert!(msg.contains("cannot be opened"), "{msg}");
	assert!(
		!msg.contains("enable --now"),
		"enabling the socket does not fix a permission problem: {msg}"
	);
}

/// The io::ErrorKind must survive, since it is what distinguishes the two.
#[cfg(unix)]
#[test]
fn the_error_kind_is_preserved() {
	let e = socket_error("/x", std::io::Error::from(std::io::ErrorKind::NotFound));
	assert!(
		matches!(&e, super::super::PodmanError::Connect(io) if io.kind() == std::io::ErrorKind::NotFound)
	);
}
