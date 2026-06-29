//! Profile filtering — determines which services run given the active profile set.

use std::collections::HashSet;

use crate::compose::types::{ComposeFile, Service};

/// Remove services excluded by the active profile set, in place.
///
/// `active` is the CLI `--profile` list, falling back to `COMPOSE_PROFILES`.
/// A profiled service that is a transitive `depends_on` target of a retained
/// service is implicitly enabled, matching docker compose — so the output never
/// carries a dangling dependency reference.
pub fn retain_active_profiles(file: &mut ComposeFile, active: &[String]) {
	retain_active_profiles_with_targets(file, active, &[]);
}

/// Like [`retain_active_profiles`], but also keeps any service named in
/// `targets` even when its profile is inactive: naming a service on the command
/// line activates its profile (docker compose), so per-service subcommands
/// (`start`, `stop`, `build`, `push`, `pull`, …) can still address it.
pub fn retain_active_profiles_with_targets(
	file: &mut ComposeFile,
	active: &[String],
	targets: &[String],
) {
	let set = active_profiles_set(active);
	let enabled = enabled_profile_services(file, &set, targets);
	file.services.retain(|name, _| enabled.contains(name));
}

/// The set of service names that should run under the active profile set.
///
/// A service is enabled when it is unprofiled, matches an active profile (or the
/// `*` wildcard), or is explicitly named in `targets` (naming a service on the
/// command line activates its profile). Implicit activation then pulls in the
/// transitive `depends_on` targets of every enabled service — even profiled
/// ones whose profile is inactive — so a started service never depends on a
/// service that was filtered out. Mirrors docker compose, which activates a
/// profiled service that is depended on by a started one.
///
/// This is the single source of truth for "which services does an `up`/`config`
/// with these profiles touch": [`retain_active_profiles_with_targets`] uses it
/// to prune the config, and the `up`/`create` lifecycle path uses it to decide
/// which services to actually start — so the two never disagree.
pub(crate) fn enabled_profile_services(
	file: &ComposeFile,
	active: &HashSet<String>,
	targets: &[String],
) -> HashSet<String> {
	let named: HashSet<&str> = targets.iter().map(|s| s.as_str()).collect();

	// Directly enabled: an unprofiled service, a profile match (or `*`), or a
	// service explicitly named on the command line.
	let mut enabled: HashSet<String> = file
		.services
		.iter()
		.filter(|(name, svc)| service_in_profiles(svc, active) || named.contains(name.as_str()))
		.map(|(name, _)| name.clone())
		.collect();

	// Implicit activation: pull in profiled `depends_on` targets of enabled
	// services, transitively, so a retained service never references a dropped
	// dependency.
	let mut stack: Vec<String> = enabled.iter().cloned().collect();
	while let Some(name) = stack.pop() {
		if let Some(svc) = file.services.get(&name) {
			for dep in svc.depends_on.service_names() {
				if file.services.contains_key(&dep) && enabled.insert(dep.clone()) {
					stack.push(dep);
				}
			}
		}
	}

	enabled
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
/// Services with no profiles always start. A literal `*` in the active set is a
/// wildcard that enables every profiled service (docker compose's
/// "enable all profiles").
pub(super) fn service_in_profiles(service: &Service, active: &HashSet<String>) -> bool {
	if service.profiles.is_empty() {
		return true;
	}
	if active.contains("*") {
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

	#[test]
	fn wildcard_enables_all_profiles() {
		// `--profile '*'` enables every profiled service, matching docker compose.
		let svc = Service {
			profiles: vec!["debug".to_string()],
			..Default::default()
		};
		let active: HashSet<String> = ["*".to_string()].into();
		assert!(service_in_profiles(&svc, &active));

		let yaml = "services:\n  \
			web:\n    image: x\n  \
			debugger:\n    image: x\n    profiles: [debug]\n  \
			db:\n    image: x\n    profiles: [prod]\n";
		let mut file = crate::parse_str(yaml).unwrap();
		retain_active_profiles(&mut file, &["*".to_string()]);
		assert_eq!(file.services.len(), 3);
	}

	#[test]
	fn implicit_activation_keeps_profiled_dependency() {
		// `app` (active) depends on `db` (profiles: [storage]). With no profile
		// active, `db` is implicitly enabled so `app` keeps a satisfiable dep —
		// no dangling reference, matching docker compose.
		let yaml = "services:\n  \
			app:\n    image: x\n    depends_on: [db]\n  \
			db:\n    image: x\n    profiles: [storage]\n";
		let mut file = crate::parse_str(yaml).unwrap();
		temp_env::with_var_unset("COMPOSE_PROFILES", || {
			retain_active_profiles(&mut file, &[]);
		});
		assert!(file.services.contains_key("app"));
		assert!(
			file.services.contains_key("db"),
			"profiled depends_on target is implicitly activated"
		);
	}

	#[test]
	fn implicit_activation_is_transitive() {
		// app -> db -> storage, where both db and storage are profiled. Enabling
		// app must pull in the whole transitive dependency chain.
		let yaml = "services:\n  \
			app:\n    image: x\n    depends_on: [db]\n  \
			db:\n    image: x\n    profiles: [p]\n    depends_on: [storage]\n  \
			storage:\n    image: x\n    profiles: [q]\n";
		let mut file = crate::parse_str(yaml).unwrap();
		temp_env::with_var_unset("COMPOSE_PROFILES", || {
			retain_active_profiles(&mut file, &[]);
		});
		assert_eq!(file.services.len(), 3);
	}

	#[test]
	fn unrelated_profiled_service_still_dropped() {
		// Implicit activation only reaches dependencies — an unrelated profiled
		// service is still removed.
		let yaml = "services:\n  \
			app:\n    image: x\n    depends_on: [db]\n  \
			db:\n    image: x\n    profiles: [storage]\n  \
			extra:\n    image: x\n    profiles: [other]\n";
		let mut file = crate::parse_str(yaml).unwrap();
		temp_env::with_var_unset("COMPOSE_PROFILES", || {
			retain_active_profiles(&mut file, &[]);
		});
		assert!(file.services.contains_key("db"));
		assert!(!file.services.contains_key("extra"));
	}

	#[test]
	fn enabled_set_activates_profiled_dependency_for_up() {
		// The `up`/`create` lifecycle path consults this set directly. `app`
		// (unprofiled, started) depends on `db` (profiles: [storage]). With no
		// profile active, `db` must be in the enabled set so `up` actually
		// creates it — otherwise `app` runs with an unsatisfied dependency.
		let yaml = "services:\n  \
			app:\n    image: x\n    depends_on: [db]\n  \
			db:\n    image: x\n    profiles: [storage]\n";
		let file = crate::parse_str(yaml).unwrap();
		let active: HashSet<String> = HashSet::new();
		let enabled = enabled_profile_services(&file, &active, &[]);
		assert!(enabled.contains("app"));
		assert!(
			enabled.contains("db"),
			"profiled depends_on target is in the started set"
		);
	}

	#[test]
	fn enabled_set_excludes_unrelated_profiled_service() {
		// Only dependencies are pulled in — an unrelated profiled service stays
		// out of the started set, so `up` does not over-activate it.
		let yaml = "services:\n  \
			app:\n    image: x\n    depends_on: [db]\n  \
			db:\n    image: x\n    profiles: [storage]\n  \
			extra:\n    image: x\n    profiles: [other]\n";
		let file = crate::parse_str(yaml).unwrap();
		let active: HashSet<String> = HashSet::new();
		let enabled = enabled_profile_services(&file, &active, &[]);
		assert!(enabled.contains("app"));
		assert!(enabled.contains("db"));
		assert!(!enabled.contains("extra"));
	}

	#[test]
	fn named_target_keeps_inactive_profile_service() {
		// Naming a profiled service on the command line activates its profile, so
		// per-service subcommands can still address it.
		let yaml = "services:\n  \
			web:\n    image: x\n  \
			debugger:\n    image: x\n    profiles: [debug]\n";
		let mut file = crate::parse_str(yaml).unwrap();
		temp_env::with_var_unset("COMPOSE_PROFILES", || {
			retain_active_profiles_with_targets(&mut file, &[], &["debugger".to_string()]);
		});
		assert!(file.services.contains_key("web"));
		assert!(file.services.contains_key("debugger"));
	}
}
