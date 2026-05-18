//! Profile filtering — determines which services run given the active profile set.

use std::collections::HashSet;

use crate::compose::types::Service;

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
