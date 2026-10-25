use super::control_escape::{escape_json_controls, escape_log_controls};

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
