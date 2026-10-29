use super::*;

/// Anchors that hold aliases to the anchor before them: `levels` steps, each
/// repeating the previous one ten times, over `width` scalars to begin with.
/// A few lines of text for `width * 10^levels` nodes.
fn nested(width: usize, levels: usize) -> String {
	let scalars = vec!["a"; width].join(", ");
	let mut yaml = format!("x-l0: &l0 [{scalars}]\n");
	for level in 1..=levels {
		let refs = vec![format!("*l{}", level - 1); 10].join(", ");
		yaml.push_str(&format!("x-l{level}: &l{level} [{refs}]\n"));
	}
	yaml.push_str("services:\n  web:\n    image: x\n");
	yaml
}

/// The text of this document passes the lexical guard (a handful of references
/// in a few kilobytes), and it still replays to millions of nodes.
#[test]
fn anchors_that_hold_aliases_are_counted_by_what_they_expand_to() {
	let yaml = nested(1000, 4);
	assert!(yaml.len() < 8 * 1024, "{} bytes", yaml.len());
	assert!(
		super::super::merge::interpolated_value(&yaml, None).is_err(),
		"the document expands to ten million nodes"
	);
}

#[test]
fn a_small_expansion_is_within_the_limit() {
	assert!(within_limit(&nested(10, 2), 10_000).is_ok());
}

#[test]
fn the_limit_is_the_nodes_replayed_not_the_text() {
	// 10 * 10 * 10 scalars plus the containers: well over a thousand nodes.
	assert!(within_limit(&nested(10, 2), 500).is_err());
	assert!(within_limit(&nested(10, 2), 100_000).is_ok());
}

/// A document without aliases costs one node per value and is not refused, and
/// a syntax error is left for the real parse to report.
#[test]
fn a_plain_document_and_a_broken_one_are_not_this_guards_to_refuse() {
	assert!(within_limit("services:\n  web:\n    image: x\n", 10).is_ok());
	assert!(within_limit("services: [unterminated\n", 10).is_ok());
}

/// Tags are in compose files (`!override`, `!reset`); the counter follows them.
#[test]
fn tagged_values_are_followed() {
	let yaml = "services:\n  web:\n    ports: !override\n      - \"80\"\n      - \"81\"\n";
	assert!(within_limit(yaml, 100).is_ok());
	assert!(within_limit(yaml, 4).is_err());
}

/// The lexical scan reads `x 'y` as an opened quote and counts no alias after
/// it, so the references-and-size limits never run. The count of replayed nodes
/// does not depend on it.
#[test]
fn a_plain_scalar_that_hides_the_aliases_from_the_scan_does_not_hide_the_expansion() {
	let yaml = format!("note: x 'y\n{}", nested(1000, 4));
	assert!(
		super::super::merge::interpolated_value(&yaml, None).is_err(),
		"the document expands to ten million nodes"
	);
}
