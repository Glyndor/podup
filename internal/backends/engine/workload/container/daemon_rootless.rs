//! Whether the Podman daemon the engine talks to runs rootless, which decides
//! if the device permission segment can be sent as a cgroup rule.

use serde::Deserialize;

use crate::engine::Engine;
use crate::libpod::API_PREFIX;

#[derive(Deserialize, Default)]
struct Info {
	#[serde(default)]
	host: Host,
}

#[derive(Deserialize, Default)]
struct Host {
	#[serde(default)]
	security: Security,
}

#[derive(Deserialize, Default)]
struct Security {
	#[serde(default)]
	rootless: bool,
}

impl Info {
	fn is_rootless(&self) -> bool {
		self.host.security.rootless
	}
}

impl Engine {
	/// Ask the daemon whether it is rootless. A daemon that does not answer is
	/// treated as rootful, so the rule is sent as before.
	pub(super) async fn daemon_is_rootless(&self) -> bool {
		self.client
			.get_json::<Info>(&format!("{API_PREFIX}/info"))
			.await
			.is_ok_and(|info| info.is_rootless())
	}
}

#[cfg(test)]
#[path = "daemon_rootless_tests.rs"]
mod tests;
