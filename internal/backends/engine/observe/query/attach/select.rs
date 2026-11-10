//! Which services an attached `up` follows.

use super::AttachOptions;
use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};

/// The services whose output is followed: those that did not opt out with
/// `attach: false`, narrowed to `options.attach` when it names any, minus
/// `options.no_attach`. A name that is not a service of `file` is an input
/// error, so a typo does not silently follow nothing.
pub(super) fn attached_services<'a>(
	file: &'a ComposeFile,
	options: &AttachOptions,
) -> Result<Vec<(&'a str, &'a Service)>> {
	for name in options.attach.iter().chain(&options.no_attach) {
		if !file.services.contains_key(name) {
			return Err(ComposeError::ServiceNotFound(name.clone()));
		}
	}
	Ok(file
		.services
		.iter()
		.filter(|(name, service)| {
			service.attach.unwrap_or(true)
				&& (options.attach.is_empty() || options.attach.contains(name))
				&& !options.no_attach.contains(name)
		})
		.map(|(name, service)| (name.as_str(), service))
		.collect())
}

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;
