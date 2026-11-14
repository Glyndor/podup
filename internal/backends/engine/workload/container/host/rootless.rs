//! Whether the Podman daemon the engine talks to runs rootless, which decides
//! if the device permission segment can be sent as a cgroup rule.

use crate::engine::Engine;

// The daemon's `GET /info` document lives with the other host facts it
// carries, so the daemon is asked once per invocation instead of once per
// question.
pub(in crate::backends::engine) use super::info::Info;

impl Engine {
	/// Ask the daemon whether it is rootless. A daemon that does not answer is
	/// treated as rootful, so the rule is sent as before.
	pub(in crate::backends::engine::workload::container) async fn daemon_is_rootless(
		&self,
	) -> bool {
		self.host_info()
			.await
			.as_ref()
			.is_some_and(Info::is_rootless)
	}
}

#[cfg(test)]
#[path = "rootless_tests.rs"]
mod tests;
