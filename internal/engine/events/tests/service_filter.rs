use super::*;

/// #2014: `--format json` lines and the table form both run through the same
/// client-side service filter (`event_in_services`), kept pure so the
/// behaviour is pinned without a fake socket. The service a container
/// belongs to is its `podup.service` label, which libpod puts under
/// `Actor.Attributes`. Without that attribute the event cannot match a named
/// service (a `network`/`volume` event that still has the project label is
/// not a container event); with it, only the matching name passes.
#[test]
fn event_in_services_event_for_web_matches_when_web_is_requested() {
	let ev = json!({
		"Type": "container",
		"Actor": { "Attributes": { "podup.service": "web", "name": "web-1" } },
	});
	assert!(
		event_in_services(&ev, &["web".to_string()]),
		"a container event with `podup.service==web` must pass when `web` is requested"
	);
}

#[test]
fn event_in_services_event_for_web_does_not_match_when_db_is_requested() {
	let ev = json!({
		"Type": "container",
		"Actor": { "Attributes": { "podup.service": "web", "name": "web-1" } },
	});
	assert!(
		!event_in_services(&ev, &["db".to_string()]),
		"a `web` event must not pass when only `db` is requested"
	);
}

#[test]
fn event_in_services_matches_when_one_of_several_services_is_requested() {
	let ev = json!({
		"Type": "container",
		"Actor": { "Attributes": { "podup.service": "web", "name": "web-1" } },
	});
	assert!(
		event_in_services(&ev, &["db".to_string(), "web".to_string()]),
		"a `web` event must pass when the requested set is `[db, web]`"
	);
}

/// An event without `podup.service` is dropped when services were given:
/// this is a `network`/`volume`/`image` event that still carries the
/// project label, not a container this project started. `docker compose
/// events web` does not print these either (#2014). The empty-services
/// case below proves the drop does NOT happen without a target.
#[test]
fn event_in_services_event_without_service_attribute_is_dropped_when_services_are_given() {
	let ev = json!({
		"Type": "network",
		"Actor": { "Attributes": { "name": "proj_default" } },
	});
	assert!(
		!event_in_services(&ev, &["web".to_string()]),
		"a network event without `podup.service` must be dropped when services were named"
	);
}

#[test]
fn event_in_services_empty_services_passes_every_event() {
	for ev in [
		json!({
			"Type": "container",
			"Actor": { "Attributes": { "podup.service": "web", "name": "web-1" } },
		}),
		json!({
			"Type": "network",
			"Actor": { "Attributes": { "name": "proj_default" } },
		}),
		json!({}),
	] {
		assert!(
			event_in_services(&ev, &[]),
			"an empty requested list must never drop an event: {ev}"
		);
	}
}
