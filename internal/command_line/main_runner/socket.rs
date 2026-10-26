//! Podman-socket resolution and connection for the CLI's commands.
//!
//! Every command that talks to Podman reaches [`connect_podman`]; the
//! connection carries the version-pinging [`connect_checked`] semantics so a
//! host below the supported floor is refused before the command sends anything
//! else.
//!
//! [`connect_checked`]: podup::podman::connect_checked

/// Resolve the Podman socket: an explicit `--socket` / `PODMAN_SOCKET` value
/// wins (clap already folds the env var into `cli.socket`); otherwise fall back
/// to `DOCKER_HOST` for Docker compatibility. A remote scheme on either is
/// rejected by [`podup::podman::connect`], so `DOCKER_HOST=tcp://…` fails closed
/// rather than being silently ignored.
pub(crate) fn resolve_socket(cli_socket: Option<&str>) -> Option<String> {
	cli_socket
		.map(str::to_string)
		.or_else(|| std::env::var("DOCKER_HOST").ok().filter(|s| !s.is_empty()))
}

/// Connect to the Podman `socket` names (or the resolved default), with the
/// pool size asked for, and confirm the engine meets the supported floor
/// before the command sends anything else. Every command that talks to
/// Podman gets its client here.
pub(crate) async fn connect_podman(
	socket: Option<&str>,
	pool_size: Option<usize>,
) -> podup::Result<podup::Client> {
	podup::podman::connect_checked(
		resolve_socket(socket).as_deref(),
		pool_size.unwrap_or(podup::Client::DEFAULT_POOL_SIZE),
	)
	.await
}
