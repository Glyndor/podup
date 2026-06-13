//! Service-selection helpers for lifecycle commands: stop grace period,
//! target-list filtering, and `depends_on` expansion.

use std::collections::HashSet;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};

pub(super) fn grace_period_secs(service: &Service) -> i32 {
	service
		.stop_grace_period
		.as_deref()
		.and_then(crate::size::parse_duration_secs)
		.and_then(|s| i32::try_from(s).ok())
		.unwrap_or(10)
}

/// Return the ordered service names filtered to `target_services`.
///
/// Returns an error if any name in `target_services` is not in the file.
pub(super) fn filter_services(
	file: &ComposeFile,
	order: Vec<String>,
	target_services: &[String],
) -> Result<Vec<String>> {
	if target_services.is_empty() {
		return Ok(order);
	}
	for name in target_services {
		if !file.services.contains_key(name) {
			return Err(ComposeError::ServiceNotFound(name.clone()));
		}
	}
	let set: std::collections::HashSet<&str> = target_services.iter().map(|s| s.as_str()).collect();
	Ok(order
		.into_iter()
		.filter(|n| set.contains(n.as_str()))
		.collect())
}

/// Resolve which services `up` should start given an explicit target list.
///
/// Returns `None` when no targets are given (start everything). Otherwise the
/// set contains the targets plus, unless `no_deps` is set, their transitive
/// `depends_on` services.
pub(super) fn expand_targets(
	file: &ComposeFile,
	target_services: &[String],
	no_deps: bool,
) -> Option<HashSet<String>> {
	if target_services.is_empty() {
		return None;
	}
	let mut set = HashSet::new();
	let mut stack: Vec<String> = target_services.to_vec();
	while let Some(name) = stack.pop() {
		if !set.insert(name.clone()) {
			continue;
		}
		if !no_deps {
			if let Some(service) = file.services.get(&name) {
				for dep in service.depends_on.service_names() {
					if !set.contains(&dep) {
						stack.push(dep);
					}
				}
			}
		}
	}
	Some(set)
}

#[cfg(test)]
mod tests {
	use super::{expand_targets, filter_services};
	use crate::compose::types::{ComposeFile, Service};

	fn file_with_services(names: &[&str]) -> ComposeFile {
		let mut file = ComposeFile::default();
		for &name in names {
			file.services.insert(name.to_string(), Service::default());
		}
		file
	}

	#[test]
	fn filter_empty_target_returns_all() {
		let file = file_with_services(&["a", "b", "c"]);
		let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
		let result = filter_services(&file, order.clone(), &[]).unwrap();
		assert_eq!(result, order);
	}

	#[test]
	fn filter_target_subset_returns_intersection() {
		let file = file_with_services(&["a", "b", "c"]);
		let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
		let result = filter_services(&file, order, &["b".to_string()]).unwrap();
		assert_eq!(result, vec!["b".to_string()]);
	}

	#[test]
	fn filter_target_preserves_order() {
		let file = file_with_services(&["a", "b", "c"]);
		let order = vec!["a".to_string(), "b".to_string(), "c".to_string()];
		let result = filter_services(&file, order, &["c".to_string(), "a".to_string()]).unwrap();
		assert_eq!(result, vec!["a".to_string(), "c".to_string()]);
	}

	#[test]
	fn filter_unknown_service_returns_error() {
		let file = file_with_services(&["a"]);
		let order = vec!["a".to_string()];
		let err = filter_services(&file, order, &["z".to_string()]).unwrap_err();
		assert!(matches!(
			err,
			crate::error::ComposeError::ServiceNotFound(_)
		));
	}

	// --- expand_targets ---

	fn file_web_depends_db() -> ComposeFile {
		crate::parse_str(
			"services:\n  db:\n    image: x\n  web:\n    image: x\n    depends_on:\n      - db\n",
		)
		.unwrap()
	}

	#[test]
	fn expand_targets_empty_is_none() {
		let file = file_web_depends_db();
		assert!(expand_targets(&file, &[], false).is_none());
	}

	#[test]
	fn expand_targets_includes_dependencies() {
		let file = file_web_depends_db();
		let set = expand_targets(&file, &["web".to_string()], false).unwrap();
		assert!(set.contains("web"));
		assert!(set.contains("db"));
	}

	#[test]
	fn expand_targets_no_deps_excludes_dependencies() {
		let file = file_web_depends_db();
		let set = expand_targets(&file, &["web".to_string()], true).unwrap();
		assert!(set.contains("web"));
		assert!(!set.contains("db"));
	}
}
