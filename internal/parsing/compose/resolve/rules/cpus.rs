//! `cpus` and `cpu_period` together are refused, as Docker and Podman do.
//!
//! `cpus` is a CFS quota over the default 100 ms period, so next to another
//! `cpu_period` it no longer means what it says: 1.5 CPUs over a 50 ms period
//! came out as 3. Docker (`Conflicting options: Nano CPUs and CPU Period cannot
//! both be set`) and Podman 5.7.0 (`--cpu-period and --cpus cannot be set
//! together`) both refuse the pair, so reading the file does too.

use crate::compose::types::Service;
use crate::error::{ComposeError, Result};

/// Refuse `cpus` (top level or `deploy.resources.limits.cpus`) next to
/// `cpu_period`. An empty `cpus` (`${CPUS:-}`) counts as unset.
pub(in crate::parsing::compose::resolve) fn validate_cpu_limits(
	name: &str,
	service: &Service,
) -> Result<()> {
	let deploy_cpus = service
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.and_then(|l| l.cpus.as_deref());
	let cpus = service
		.cpus
		.as_deref()
		.or(deploy_cpus)
		.filter(|c| !c.trim().is_empty());
	if cpus.is_some() && service.cpu_period.is_some() {
		return Err(ComposeError::Invalid(format!(
			"service '{}': cpus and cpu_period cannot be set together (Docker and Podman \
			 refuse it too); use cpu_quota with cpu_period, or cpus alone",
			crate::error::sanitize_name(name)
		)));
	}
	Ok(())
}

#[cfg(test)]
#[path = "cpus_tests.rs"]
mod tests;
