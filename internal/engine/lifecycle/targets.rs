//! Service-selection helpers for lifecycle commands: stop grace period,
//! target-list filtering, and `depends_on` expansion.

use std::collections::HashSet;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};

/// The per-service shutdown grace from `stop_grace_period` (default 10s).
pub(super) fn service_grace_period_secs(service: &Service) -> i32 {
	service
		.stop_grace_period
		.as_deref()
		.and_then(crate::size::parse_duration_secs)
		.and_then(|s| i32::try_from(s).ok())
		.unwrap_or(10)
}

impl crate::engine::Engine {
	/// Shutdown grace (seconds) for a service: the CLI `-t/--timeout` override
	/// when set, otherwise the service's `stop_grace_period`.
	pub(super) fn grace_period_secs(&self, service: &Service) -> i32 {
		self.stop_timeout
			.unwrap_or_else(|| service_grace_period_secs(service))
	}
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

/// Error if any requested target service name is absent from the file.
///
/// The up/create path expands targets into a set without checking membership, so
/// a bogus name would silently match nothing and exit 0. This validates the list
/// up front — matching docker-compose and the stop/start/kill commands, which
/// already reject unknown services via [`filter_services`].
pub(super) fn validate_targets(file: &ComposeFile, target_services: &[String]) -> Result<()> {
	for name in target_services {
		if !file.services.contains_key(name) {
			return Err(ComposeError::ServiceNotFound(name.clone()));
		}
	}
	Ok(())
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

/// Whether `name` is part of the started set described by `target_set`.
///
/// `target_set` is `None` when no explicit target list was given (every
/// service is in scope, so the answer is always `true`). Otherwise a name is
/// "started" only if it is present in the set. Under `up --no-deps`,
/// [`expand_targets`] omits the targets' dependencies, so this returns `false`
/// for an intentionally-excluded dependency — letting the caller skip its
/// `depends_on` readiness wait, matching docker-compose.
pub(super) fn in_started_set(target_set: &Option<HashSet<String>>, name: &str) -> bool {
	match target_set {
		None => true,
		Some(set) => set.contains(name),
	}
}

#[cfg(test)]
mod tests {
	use super::{
		expand_targets, filter_services, in_started_set, service_grace_period_secs,
		validate_targets,
	};
	use crate::compose::types::{ComposeFile, Service};
	use std::collections::HashSet;

	// --- service_grace_period_secs ---

	#[test]
	fn grace_period_defaults_to_ten_seconds() {
		// No stop_grace_period set → the docker-compose default of 10s.
		assert_eq!(service_grace_period_secs(&Service::default()), 10);
	}

	#[test]
	fn grace_period_parses_duration() {
		// Plain seconds and a single-unit minutes value both resolve.
		let svc = Service {
			stop_grace_period: Some("90s".to_string()),
			..Default::default()
		};
		assert_eq!(service_grace_period_secs(&svc), 90);

		let svc = Service {
			stop_grace_period: Some("2m".to_string()),
			..Default::default()
		};
		assert_eq!(service_grace_period_secs(&svc), 120);
	}

	#[test]
	fn grace_period_falls_back_on_unparseable() {
		// A value that does not parse as a duration falls back to the default.
		let svc = Service {
			stop_grace_period: Some("not-a-duration".to_string()),
			..Default::default()
		};
		assert_eq!(service_grace_period_secs(&svc), 10);
	}

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

	// --- validate_targets ---

	#[test]
	fn validate_targets_empty_is_ok() {
		let file = file_with_services(&["a", "b"]);
		assert!(validate_targets(&file, &[]).is_ok());
	}

	#[test]
	fn validate_targets_known_names_ok() {
		let file = file_with_services(&["a", "b"]);
		assert!(validate_targets(&file, &["a".to_string(), "b".to_string()]).is_ok());
	}

	#[test]
	fn validate_targets_unknown_name_errors() {
		// An `up`/`create` for a service the file does not define must error
		// rather than silently match nothing and exit 0.
		let file = file_with_services(&["a"]);
		let err = validate_targets(&file, &["no-such-service".to_string()]).unwrap_err();
		assert!(matches!(
			err,
			crate::error::ComposeError::ServiceNotFound(name) if name == "no-such-service"
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

	// --- in_started_set ---

	#[test]
	fn in_started_set_none_is_always_true() {
		// No explicit target list: every service (including any dependency)
		// is in scope, so the readiness wait is never skipped.
		assert!(in_started_set(&None, "anything"));
	}

	#[test]
	fn in_started_set_member_is_true() {
		let set: HashSet<String> = ["web".to_string(), "db".to_string()].into_iter().collect();
		assert!(in_started_set(&Some(set), "db"));
	}

	#[test]
	fn in_started_set_excluded_dep_is_false() {
		// Mirrors `up web --no-deps`: `expand_targets` yields {web} only, so the
		// excluded dependency `db` is not in the started set and its readiness
		// wait must be skipped.
		let file = file_web_depends_db();
		let target_set = expand_targets(&file, &["web".to_string()], true);
		assert!(in_started_set(&target_set, "web"));
		assert!(!in_started_set(&target_set, "db"));
	}

	#[test]
	fn in_started_set_partial_target_includes_transitive_dep() {
		// `up web` (without --no-deps) pulls `db` into the set, so its readiness
		// wait is still honored.
		let file = file_web_depends_db();
		let target_set = expand_targets(&file, &["web".to_string()], false);
		assert!(in_started_set(&target_set, "web"));
		assert!(in_started_set(&target_set, "db"));
	}
}
