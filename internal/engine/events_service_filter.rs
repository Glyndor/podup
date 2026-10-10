//! Per-event service filter for the events feed.
//!
//! The filter runs in podup, not as libpod `label=` filters, because
//! libpod ANDs them, so two `podup.service=...` predicates would match
//! nothing. The container's service is its `podup.service` label, which
//! libpod puts under `Actor.Attributes["podup.service"]`. An event
//! without that label (a `network`/`volume` event that still carries
//! the project label) is dropped when services are given, because it is
//! not a container this project started. Lives in its own module so
//! `events.rs` stays under the 500-line source budget (#2014).

use serde_json::Value;

/// Return `true` when the parsed event should be emitted for a feed
/// filtered to `services`. Pure.
pub(crate) fn event_in_services(value: &Value, services: &[String]) -> bool {
	if services.is_empty() {
		return true;
	}
	let Some(svc) = value
		.pointer("/Actor/Attributes/podup.service")
		.and_then(Value::as_str)
	else {
		return false;
	};
	services.iter().any(|s| s == svc)
}
