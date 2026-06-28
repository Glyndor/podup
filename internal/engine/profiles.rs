//! Profile filtering — determines which services run given the active profile set.

use std::collections::HashSet;

use crate::compose::types::{ComposeFile, Service};

/// Remove services excluded by the active profile set, in place.
///
/// Mirrors what `up` actually starts (a per-service profile match, with no
/// implicit activation of a profiled `depends_on` target), so `config` presents
/// the same service set the runtime would bring up. `active` is the CLI
/// `--profile` list, falling back to `COMPOSE_PROFILES`.
pub fn retain_active_profiles(file: &mut ComposeFile, active: &[String]) {
	let set = active_profiles_set(active);
	file.services
		.retain(|_, svc| service_in_profiles(svc, &set));
}

/// Build the active-profile set, falling back to `COMPOSE_PROFILES` env var.
pub(super) fn active_profiles_set(active: &[String]) -> HashSet<String> {
	if !active.is_empty() {
		return active.iter().cloned().collect();
	}
	std::env::var("COMPOSE_PROFILES")
		.ok()
		.map(|s| {
			s.split(',')
				.map(|p| p.trim().to_string())
				.filter(|p| !p.is_empty())
				.collect()
		})
		.unwrap_or_default()
}

/// True if the service should be started given the active profile set.
///
/// Services with no profiles always start.
pub(super) fn service_in_profiles(service: &Service, active: &HashSet<String>) -> bool {
	if service.profiles.is_empty() {
		return true;
	}
	service.profiles.iter().any(|p| active.contains(p))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::Service;

	#[test]
	fn explicit_profiles_ignores_env() {
		let set = active_profiles_set(&["prod".to_string()]);
		assert!(set.contains("prod"));
		assert_eq!(set.len(), 1);
	}

	#[test]
	fn empty_slice_with_no_env_returns_empty() {
		// Scope COMPOSE_PROFILES to "unset" race-free: `temp-env` serializes
		// the mutation and restores the prior value, avoiding the data race
		// that a bare `std::env::remove_var` carries under the parallel test
		// runner.
		temp_env::with_var_unset("COMPOSE_PROFILES", || {
			let set = active_profiles_set(&[]);
			assert!(set.is_empty());
		});
	}

	#[test]
	fn empty_slice_falls_back_to_env_var() {
		// With no explicit profiles, COMPOSE_PROFILES is parsed: comma-separated,
		// each entry trimmed, empty entries dropped.
		temp_env::with_var("COMPOSE_PROFILES", Some(" debug , , prod "), || {
			let set = active_profiles_set(&[]);
			assert_eq!(set.len(), 2);
			assert!(set.contains("debug"));
			assert!(set.contains("prod"));
		});
	}

	#[test]
	fn service_with_no_profiles_always_runs() {
		let svc = Service::default();
		let active: HashSet<String> = HashSet::new();
		assert!(service_in_profiles(&svc, &active));
	}

	#[test]
	fn service_profile_matches_active() {
		let svc = Service {
			profiles: vec!["debug".to_string()],
			..Default::default()
		};
		let active: HashSet<String> = ["debug".to_string()].into();
		assert!(service_in_profiles(&svc, &active));
	}

	#[test]
	fn service_profile_does_not_match() {
		let svc = Service {
			profiles: vec!["debug".to_string()],
			..Default::default()
		};
		let active: HashSet<String> = ["prod".to_string()].into();
		assert!(!service_in_profiles(&svc, &active));
	}

	#[test]
	fn service_any_profile_match_sufficient() {
		let svc = Service {
			profiles: vec!["debug".to_string(), "prod".to_string()],
			..Default::default()
		};
		let active: HashSet<String> = ["prod".to_string()].into();
		assert!(service_in_profiles(&svc, &active));
	}

	#[test]
	fn retain_active_profiles_keeps_unprofiled_and_active() {
		let yaml = "services:\n  \
			web:\n    image: x\n  \
			debugger:\n    image: x\n    profiles: [debug]\n  \
			db:\n    image: x\n    profiles: [prod]\n";
		// With `debug` active: the unprofiled `web` and the `debug` service stay,
		// the `prod`-only `db` is dropped — exactly what `up --profile debug` runs.
		let mut file = crate::parse_str(yaml).unwrap();
		retain_active_profiles(&mut file, &["debug".to_string()]);
		assert!(file.services.contains_key("web"));
		assert!(file.services.contains_key("debugger"));
		assert!(!file.services.contains_key("db"));

		// With no active profiles, every profiled service is dropped.
		let mut file = crate::parse_str(yaml).unwrap();
		temp_env::with_var_unset("COMPOSE_PROFILES", || {
			retain_active_profiles(&mut file, &[]);
		});
		assert!(file.services.contains_key("web"));
		assert_eq!(file.services.len(), 1);
	}
}
