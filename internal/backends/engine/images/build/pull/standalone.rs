//! The pull policy a standalone `pull` applies when `--policy` is absent.

use super::{PullPolicy, Service};
use crate::engine::Engine;
use crate::error::Result;

impl Engine {
	/// Resolve the policy for the `pull` subcommand. Asking to pull means
	/// asking the registry, so a service left on the implicit or explicit
	/// `missing` policy is refreshed instead of skipped when its image is
	/// already local, which is what lets `pull && up -d` move a mutable tag.
	/// A `--policy` override, `never`, `build` and the other explicit values
	/// keep their meaning. A service that builds its own image and sets no
	/// policy keeps `missing`, because its tag may not exist in any registry.
	pub(in crate::backends::engine) fn resolved_standalone_pull_policy(
		&self,
		service_name: &str,
		service: &Service,
	) -> Result<PullPolicy> {
		let resolved = self.resolved_pull_policy(service_name, service)?;
		if self.pull_policy_override.is_some() || resolved != PullPolicy::Fixed("missing") {
			return Ok(resolved);
		}
		let refreshable = match service.pull_policy.as_deref() {
			None => service.build.is_none(),
			Some("missing" | "if_not_present") => true,
			Some(_) => false,
		};
		Ok(if refreshable {
			PullPolicy::Fixed("always")
		} else {
			resolved
		})
	}
}
