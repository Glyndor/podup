//! `build.additional_contexts` entries that name another service.
//!
//! A value of `service:<name>` is the image that service builds, not a
//! directory. The service has to be built first, and the context reaches the
//! daemon as that image.

use std::collections::HashSet;

use crate::compose::types::{BuildConfig, ComposeFile};
use crate::error::{ComposeError, Result};

use super::primary_build_tag;

/// The service a context value names, when it names one.
fn referenced_service(value: &str) -> Option<&str> {
	value.strip_prefix("service:")
}

/// The additional contexts of `build`, with each `service:<name>` value
/// replaced by the image that service produces.
///
/// A service that is not in the file, or the service itself, is an error: it
/// can never be built before itself.
pub(in crate::backends::engine::images::build) fn resolve_additional_contexts(
	project: &str,
	file: &ComposeFile,
	service_name: &str,
	build: &BuildConfig,
) -> Result<Vec<(String, String)>> {
	build
		.additional_contexts()
		.into_iter()
		.map(|(name, value)| {
			let Some(target) = referenced_service(&value) else {
				return Ok((name, value));
			};
			let service = file.services.get(target).ok_or_else(|| {
				ComposeError::Build(format!(
					"service \"{service_name}\": additional context \"{name}\" names the \
					 service \"{target}\", which is not defined"
				))
			})?;
			if target == service_name {
				return Err(ComposeError::Build(format!(
					"service \"{service_name}\": additional context \"{name}\" names the \
					 service itself"
				)));
			}
			let tags = service.build.as_ref().map(|b| b.tags()).unwrap_or(&[]);
			let image = primary_build_tag(project, target, service.image.as_deref(), tags);
			Ok((name, format!("docker-image://{image}")))
		})
		.collect()
}

/// `names` with every service their `service:` contexts name added, each one
/// ahead of the service that uses it.
///
/// A referenced service with no `build:` is a plain image and is not added.
pub(crate) fn with_context_builds(file: &ComposeFile, names: &[String]) -> Result<Vec<String>> {
	fn visit(
		file: &ComposeFile,
		name: &str,
		done: &mut HashSet<String>,
		path: &mut Vec<String>,
		out: &mut Vec<String>,
	) -> Result<()> {
		if done.contains(name) {
			return Ok(());
		}
		if path.iter().any(|seen| seen == name) {
			return Err(ComposeError::Build(format!(
				"additional contexts of the services {} depend on each other",
				path.join(", ")
			)));
		}
		let Some(build) = file.services.get(name).and_then(|s| s.build.as_ref()) else {
			return Ok(());
		};
		path.push(name.to_string());
		for (_, value) in build.additional_contexts() {
			if let Some(target) = referenced_service(&value) {
				visit(file, target, done, path, out)?;
			}
		}
		path.pop();
		done.insert(name.to_string());
		out.push(name.to_string());
		Ok(())
	}

	let mut out = Vec::new();
	let mut done = HashSet::new();
	for name in names {
		visit(file, name, &mut done, &mut Vec::new(), &mut out)?;
	}
	Ok(out)
}

#[cfg(test)]
#[path = "service_contexts_tests.rs"]
mod tests;
