//! `scale` and `deploy.replicas` both say how many containers a service runs,
//! so a file that sets them to different numbers has no single answer.

use crate::compose::types::Service;
use crate::error::{ComposeError, Result};

/// Reject a service whose `scale` and `deploy.replicas` disagree. Setting both
/// to the same number is fine, and either one alone is the whole answer.
pub(in crate::parsing::compose::resolve) fn validate_replica_counts(
	name: &str,
	service: &Service,
) -> Result<()> {
	let replicas = service.deploy.as_ref().and_then(|d| d.replicas);
	match (service.scale, replicas) {
		(Some(scale), Some(replicas)) if scale != replicas => Err(ComposeError::Invalid(format!(
			"service '{}': can't set distinct values on 'scale' ({scale}) and \
				 'deploy.replicas' ({replicas}); keep one or make them equal",
			crate::error::sanitize_name(name)
		))),
		_ => Ok(()),
	}
}

#[cfg(test)]
#[path = "replicas_tests.rs"]
mod tests;
