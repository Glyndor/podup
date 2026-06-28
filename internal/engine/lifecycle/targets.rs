//! Service-selection helpers for lifecycle commands: stop grace period,
//! target-list filtering, and `depends_on` expansion.

use std::collections::HashSet;
use std::time::Duration;

use crate::compose::types::{ComposeFile, Service};
use crate::error::{ComposeError, Result};

/// Extra wall-clock slack, beyond the grace period, before podup gives up on a
/// stalled libpod `stop` and escalates to a client-side `SIGKILL`. Podman stops
/// a container by sending `SIGTERM`, waiting the grace window, then `SIGKILL`
/// itself — so a healthy stop returns at most ~`grace` seconds in. The buffer
/// absorbs daemon/reap latency so a slow-but-working stop is never escalated;
/// anything past it means the libpod call is wedged and we kill independently.
const STOP_GRACE_BUFFER_SECS: u64 = 30;

/// The per-service shutdown grace from `stop_grace_period` (default 10s).
pub(super) fn service_grace_period_secs(service: &Service) -> i32 {
	service
		.stop_grace_period
		.as_deref()
		.and_then(crate::size::parse_duration_secs)
		.and_then(|s| i32::try_from(s).ok())
		.unwrap_or(10)
}

/// Validate a CLI `-t/--timeout` value at the trust boundary.
///
/// `-1` (docker's "wait indefinitely") and any non-negative second count are
/// accepted; anything below `-1` is rejected with [`ComposeError::InvalidTimeout`]
/// so it never reaches libpod as a `?t=<negative>` that surfaces a raw `HTTP 400`.
/// Pure so the boundary check is unit-tested without a live socket.
pub fn validate_stop_timeout(timeout: Option<i32>) -> Result<Option<i32>> {
	match timeout {
		Some(t) if t < -1 => Err(ComposeError::InvalidTimeout(t)),
		other => Ok(other),
	}
}

/// The libpod `?t=` value for a grace period. A non-negative grace passes through;
/// `-1` ("wait indefinitely") maps to the largest value libpod accepts so podman
/// does not escalate to `SIGKILL` on its own, matching `docker stop -t -1`. Pure.
pub(super) fn stop_timeout_param(grace: i32) -> i64 {
	if grace < 0 {
		i64::from(i32::MAX)
	} else {
		i64::from(grace)
	}
}

/// Client-side deadline for a `stop` call: the grace window plus
/// [`STOP_GRACE_BUFFER_SECS`]. `-1` ("wait indefinitely") yields `None`, leaving
/// the call uncapped like `docker stop -t -1`. Pure so the policy is unit-tested.
pub(super) fn stop_deadline(grace: i32) -> Option<Duration> {
	if grace < 0 {
		None
	} else {
		Some(Duration::from_secs(grace as u64 + STOP_GRACE_BUFFER_SECS))
	}
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
		expand_targets, filter_services, in_started_set, service_grace_period_secs, stop_deadline,
		stop_timeout_param, validate_stop_timeout, STOP_GRACE_BUFFER_SECS,
	};
	use crate::compose::types::{ComposeFile, Service};
	use crate::error::ComposeError;
	use std::collections::HashSet;
	use std::time::Duration;

	// --- validate_stop_timeout (#778) ---

	#[test]
	fn validate_stop_timeout_accepts_none_zero_and_positive() {
		assert_eq!(validate_stop_timeout(None).unwrap(), None);
		assert_eq!(validate_stop_timeout(Some(0)).unwrap(), Some(0));
		assert_eq!(validate_stop_timeout(Some(30)).unwrap(), Some(30));
	}

	#[test]
	fn validate_stop_timeout_accepts_minus_one_infinite() {
		// -1 is docker's "wait indefinitely" sentinel and must pass through.
		assert_eq!(validate_stop_timeout(Some(-1)).unwrap(), Some(-1));
	}

	#[test]
	fn validate_stop_timeout_rejects_below_minus_one() {
		// A value below -1 is rejected here rather than leaking a raw libpod 400.
		let err = validate_stop_timeout(Some(-2)).unwrap_err();
		assert!(matches!(err, ComposeError::InvalidTimeout(-2)));
		assert!(validate_stop_timeout(Some(-100)).is_err());
	}

	// --- stop_timeout_param (#778) ---

	#[test]
	fn stop_timeout_param_passes_through_non_negative() {
		assert_eq!(stop_timeout_param(0), 0);
		assert_eq!(stop_timeout_param(10), 10);
	}

	#[test]
	fn stop_timeout_param_maps_infinite_to_max() {
		// -1 (infinite) maps to the largest value libpod accepts so podman never
		// escalates to SIGKILL on its own, matching `docker stop -t -1`.
		assert_eq!(stop_timeout_param(-1), i64::from(i32::MAX));
	}

	// --- stop_deadline (#719) ---

	#[test]
	fn stop_deadline_is_grace_plus_buffer() {
		assert_eq!(
			stop_deadline(10),
			Some(Duration::from_secs(10 + STOP_GRACE_BUFFER_SECS))
		);
		assert_eq!(
			stop_deadline(0),
			Some(Duration::from_secs(STOP_GRACE_BUFFER_SECS))
		);
	}

	#[test]
	fn stop_deadline_infinite_is_none() {
		// -1 leaves the stop uncapped (docker `stop -t -1` parity).
		assert_eq!(stop_deadline(-1), None);
	}

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
