//! Socket-error mapping for the libpod transport: turn a bare `io::Error` from
//! the underlying connect call into a `PodmanError::Connect` whose message
//! names the socket path and suggests the operator-side fix for the two
//! cases that show up in the field (#1146).
//!
//! Split from `mod.rs` so the function and its platform-specific guidance sit
//! next to each other and leave `mod.rs` to focus on the index plus `Client`.

/// Attach the socket path and a way forward to a connection failure.
///
/// The operator saw `podman socket connection error: No such file or directory
/// (os error 2)`: no path, no distinction between "it is not there" and "I
/// cannot open it", and nothing to do about it. Everything needed was already
/// in hand one call earlier (#1146).
///
/// The path is folded into the `io::Error`'s message rather than into a new
/// error variant so `PodmanError` keeps its shape: it is public API, frozen
/// since 2.0.0. `kind()` survives, which is what tells the two cases apart.
///
/// Unix only because the hints are: `systemctl --user` means nothing to a
/// `podman machine` install, and the named-pipe connect path reports its
/// errors raw.
#[cfg(unix)]
pub(crate) fn socket_error(path: &str, e: std::io::Error) -> super::PodmanError {
	let hint = match e.kind() {
		std::io::ErrorKind::NotFound => {
			": the Podman API socket is not listening. podman itself is daemonless \
			 and needs no socket, but podup speaks the libpod API and does. Enable it \
			 with `systemctl --user enable --now podman.socket`, or for an account \
			 with no login shell: `sudo -u <user> env XDG_RUNTIME_DIR=/run/user/$(id \
			 -u <user>) systemctl --user enable --now podman.socket`"
		}
		std::io::ErrorKind::PermissionDenied => {
			": the socket exists but cannot be opened. Check that it is owned by \
			 the user running podup; a socket created by another account is not \
			 shared"
		}
		_ => "",
	};
	super::PodmanError::Connect(std::io::Error::new(e.kind(), format!("{path}: {e}{hint}")))
}
