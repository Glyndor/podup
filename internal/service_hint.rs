//! Suggest the compose service when a rejected name identifies its container.

use crate::compose::types::ComposeFile;
use crate::error::{sanitize_name, ComposeError};

/// Explain a missing service name that matches an explicit container name or
/// a project-scoped replica name. Explicit names take precedence; ties follow
/// compose-file order. All interpolated names are escaped for terminal output.
pub fn hint_for(err: &ComposeError, file: &ComposeFile, project: &str) -> Option<String> {
	let ComposeError::ServiceNotFound(name) = err.innermost() else {
		return None;
	};
	let service = file
		.services
		.iter()
		.find(|(_, service)| service.container_name.as_deref() == Some(name.as_str()))
		.map(|(service, _)| service)
		.or_else(|| {
			let (base, index) = name.rsplit_once('-')?;
			if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
				return None;
			}
			let service = base.strip_prefix(project)?.strip_prefix('-')?;
			file.services.get_key_value(service).map(|(name, _)| name)
		})?;
	Some(format!(
		"hint: '{}' is the container of service '{}'; pass the service name instead",
		sanitize_name(name),
		sanitize_name(service)
	))
}

#[cfg(test)]
#[path = "service_hint_tests.rs"]
mod tests;
