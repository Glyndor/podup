//! The Podman socket a generated unit carries, and the forms a unit can
//! represent.
//!
//! A unit runs with no environment of the operator's, so the socket selected
//! at install time has to be written into the command itself. Without it the
//! boot path talks to the default instance, which is a different one whenever
//! the operator pointed `--socket` somewhere else.

/// The value a service-mode unit passes to `podup --socket`.
///
/// podup accepts a local socket as a plain path or with its `unix://` /
/// `npipe://` scheme, and refuses a remote one. A unit carrying a remote
/// socket would fail at boot with nothing to say why, so it is refused here,
/// where the operator is watching.
pub(in super::super) fn for_podup(socket: &str) -> Result<String, String> {
	if socket.contains("://") && !socket.starts_with("unix://") && !socket.starts_with("npipe://") {
		return Err(format!(
			"socket {socket:?} cannot be represented in an autostart unit: podup talks to a \
			 local socket only"
		));
	}
	Ok(socket.to_string())
}

/// The URL a start-mode unit passes to `podman --url`.
///
/// Podman wants a URL rather than a path, so a plain path becomes
/// `unix://<path>`. A remote scheme cannot be represented either.
pub(in super::super) fn for_podman(socket: &str) -> Result<String, String> {
	let local = for_podup(socket)?;
	if local.starts_with("unix://") || local.starts_with("npipe://") {
		return Ok(local);
	}
	Ok(format!("unix://{local}"))
}
