use super::*;

/// #1078: `!override` takes the overriding value whole instead of appending.
/// Both tags were accepted and silently ignored, so a file asking for
/// replacement got a merge, the opposite of what it said.
#[test]
fn override_tag_replaces_instead_of_appending() {
	let base =
		parse_str("services:\n  web:\n    image: alpine\n    ports: [\"8080:80\"]\n").unwrap();
	let over = parse_str("services:\n  web:\n    ports: !override [\"9090:80\"]\n").unwrap();
	let raw: serde_yaml::Value =
		serde_yaml::from_str("services:\n  web:\n    ports: !override [\"9090:80\"]\n").unwrap();
	let directives = crate::compose::tags::collect(&raw);

	let merged = crate::compose::extends::merge_service_tagged(
		base.services["web"].clone(),
		over.services["web"].clone(),
		directives.get("web"),
	);
	assert_eq!(
		merged.ports.len(),
		1,
		"!override must replace, not append: {:?}",
		merged.ports
	);
}

/// `!reset` drops the key entirely, leaving the type's default.
#[test]
fn reset_tag_clears_the_base_value() {
	let base = parse_str("services:\n  web:\n    image: alpine\n    dns: [\"1.1.1.1\"]\n").unwrap();
	let over = parse_str("services:\n  web:\n    dns: !reset []\n").unwrap();
	let raw: serde_yaml::Value =
		serde_yaml::from_str("services:\n  web:\n    dns: !reset []\n").unwrap();
	let directives = crate::compose::tags::collect(&raw);

	let merged = crate::compose::extends::merge_service_tagged(
		base.services["web"].clone(),
		over.services["web"].clone(),
		directives.get("web"),
	);
	assert!(
		merged.dns.to_list().is_empty(),
		"!reset must clear the base: {:?}",
		merged.dns
	);
}

/// Without a tag the ordinary merge rule still applies; the tags must not
/// change the default behaviour of anything they are not on.
#[test]
fn an_untagged_key_still_merges_normally() {
	let base = parse_str("services:\n  web:\n    image: alpine\n    dns: [\"1.1.1.1\"]\n").unwrap();
	let over = parse_str("services:\n  web:\n    dns: [\"9.9.9.9\"]\n").unwrap();
	let merged = crate::compose::extends::merge_service_tagged(
		base.services["web"].clone(),
		over.services["web"].clone(),
		None,
	);
	assert_eq!(merged.dns.to_list(), vec!["1.1.1.1", "9.9.9.9"]);
}
