//! Validate pull intent before resource reconciliation and warn once per service.

use crate::compose::types::ComposeFile;
use crate::engine::Engine;
use crate::error::Result;

impl Engine {
	/// Validate the entire batch before emitting warnings or contacting libpod.
	pub(crate) fn pull_preflight(&self, file: &ComposeFile) -> Result<()> {
		super::build::validate_pull_fields(file)?;
		for (name, service) in &file.services {
			self.resolved_pull_policy(name, service)?;
		}
		for (name, service) in &file.services {
			if let Some(policy) = self
				.pull_policy_override
				.as_deref()
				.or(service.pull_policy.as_deref())
				.filter(|p| super::build::is_time_based_pull_policy(p))
				.filter(|_| {
					self.pull_policy_override.is_some()
						|| !matches!(
							service.podman_autoupdate(),
							Ok(Some(crate::compose::types::AutoUpdate::Registry))
						)
				}) {
				let first = self
					.pull_policy_warned
					.lock()
					.unwrap_or_else(|e| e.into_inner())
					.insert(name.clone());
				if first {
					tracing::warn!(service = %name, "{}", super::build::pull_policy_warning(policy));
				}
			}
		}
		Ok(())
	}
}

#[cfg(test)]
#[path = "pull_preflight_tests.rs"]
mod pull_preflight_tests;

#[cfg(test)]
#[path = "strict_policy_tests.rs"]
mod strict_policy_tests;
