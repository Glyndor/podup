use super::super::control_escape::{
	escape_json_controls, escape_log_controls, mark_yaml_unsafe, unmark_yaml,
};

#[test]
fn log_text_without_controls_is_borrowed_unchanged() {
	let s = "plain name\twith tab\nand line";
	assert!(matches!(
		escape_log_controls(s),
		std::borrow::Cow::Borrowed(_)
	));
	assert_eq!(escape_log_controls("héllo ✓"), "héllo ✓");
}

#[test]
fn log_text_escapes_cr_nul_esc_del_bel_and_c1() {
	let out = escape_log_controls("a\rb\u{0}c\u{1b}[2Jd\u{7f}e\u{7}f\u{9b}g\u{85}");
	assert_eq!(out, "a\\rb\\u{0}c\\u{1b}[2Jd\\u{7f}e\\u{7}f\\u{9b}g\\u{85}");
	assert!(!out.chars().any(|c| c.is_control()));
}

#[test]
fn log_text_keeps_line_feed_and_tab() {
	assert_eq!(escape_log_controls("a\nb\tc\r"), "a\nb\tc\\r");
}

#[test]
fn json_del_and_c1_become_unicode_escapes_that_decode_to_the_same_value() {
	let original = serde_json::json!({"k\u{7f}": "x\u{9b}2J\u{85}y", "plain": "ok é"});
	let rendered = serde_json::to_string_pretty(&original).unwrap();
	assert!(rendered.contains('\u{9b}'), "serde_json leaves C1 raw");
	let escaped = escape_json_controls(&rendered);
	assert!(!escaped.chars().any(|c| ('\u{7f}'..='\u{9f}').contains(&c)));
	let back: serde_json::Value = serde_json::from_str(&escaped).unwrap();
	assert_eq!(back, original);
}

#[test]
fn json_without_those_characters_is_borrowed() {
	assert!(matches!(
		escape_json_controls("{\"a\": \"é✓\"}"),
		std::borrow::Cow::Borrowed(_)
	));
}

#[test]
fn log_text_escapes_bidi_and_zero_width_characters() {
	let out = escape_log_controls("a\u{202e}b\u{2066}c\u{200b}d\u{feff}e\u{61c}f\u{200d}");
	assert_eq!(
		out,
		"a\\u{202e}b\\u{2066}c\\u{200b}d\\u{feff}e\\u{61c}f\\u{200d}"
	);
	assert_eq!(
		escape_log_controls("caf\u{e9} \u{4e2d}\u{6587}"),
		"caf\u{e9} \u{4e2d}\u{6587}"
	);
}

#[test]
fn json_bidi_and_zero_width_become_unicode_escapes_that_decode_to_the_same_value() {
	let original = serde_json::json!({"k\u{202e}": "x\u{200b}y\u{2066}", "plain": "ok \u{e9}"});
	let rendered = serde_json::to_string_pretty(&original).unwrap();
	let escaped = escape_json_controls(&rendered);
	assert!(!escaped
		.chars()
		.any(|c| matches!(c, '\u{202e}' | '\u{200b}' | '\u{2066}')));
	assert!(escaped.contains("\\u202e"));
	let back: serde_json::Value = serde_json::from_str(&escaped).unwrap();
	assert_eq!(back, original);
}

#[test]
fn yaml_bidi_and_zero_width_are_written_as_escapes_and_read_back_the_same() {
	let mut value: serde_yaml::Value =
		serde_yaml::from_str("a\u{202e}b: [\"x\u{200b}y\", \"plain\"]").unwrap();
	let original = value.clone();
	mark_yaml_unsafe(&mut value);
	let text = unmark_yaml(&serde_yaml::to_string(&value).unwrap());
	assert!(
		!text
			.chars()
			.any(|c| matches!(c, '\u{202e}' | '\u{200b}' | '\u{1}')),
		"{text}"
	);
	assert!(
		text.contains("\\u202E") && text.contains("\\u200B"),
		"{text}"
	);
	assert_eq!(
		serde_yaml::from_str::<serde_yaml::Value>(&text).unwrap(),
		original
	);
}
