//! The shapes the rule has to catch, and the ones it must leave alone.
//!
//! Every case here is a snippet of Rust the detector is asked about, and the
//! expected answer is written next to it. The control matters as much as the
//! cases: a rule that found everything would pass the first test and be
//! worthless, and the second test is what says it is not that.
//!
//! The snippets are raw strings, so the sweep that runs over this file sees
//! them as data and not as code. That is the point of the lexer: prose about
//! a shape is not a shape.

use super::detector;
use super::lexer;

/// The shapes that turn a serialisation failure into an empty or a default
/// value. Each of them is a defect, and the reason is in the comment.
const FORBIDDEN: &[(&str, &str)] = &[
	(
		"the default on the same line",
		r#"let a = serde_json::to_string(&v).unwrap_or_default();"#,
	),
	(
		"the default on the next line, which is how a long call is written",
		r#"let a = serde_json::to_string_pretty(&v)
    .unwrap_or_default();"#,
	),
	(
		"a closure that returns an empty string",
		r#"let a = serde_json::to_string(&v).unwrap_or_else(|_| String::new());"#,
	),
	(
		"a closure that returns a message instead of the failure",
		r#"let a = serde_json::to_string(&v).unwrap_or_else(|e| format!("{e}"));"#,
	),
	(
		"an empty string through `into`",
		r#"let a = serde_json::to_string(&v).unwrap_or("".into());"#,
	),
	(
		"an empty string through `to_string`",
		r#"let a = serde_json::to_string(&v).unwrap_or("".to_string());"#,
	),
	(
		"an empty string through `String::from`",
		r#"let a = serde_json::to_string(&v).unwrap_or(String::from(""));"#,
	),
	(
		"a default arm that is empty",
		r#"let a = serde_json::to_string(&v).map_or(String::new(), |t| t);"#,
	),
	(
		"the error thrown away before the default",
		r#"let a = serde_json::to_string(&v).ok().unwrap_or_default();"#,
	),
	(
		"two links in the chain",
		r#"let a = serde_json::to_string(&v).map_err(|e| e.to_string()).unwrap_or_default();"#,
	),
	(
		"the call split over several lines",
		r#"let a = serde_json::to_string(
    &v,
)
.unwrap_or_default();"#,
	),
	(
		"the YAML encoder",
		r#"let a = serde_yaml::to_string(&v).unwrap_or_default();"#,
	),
	(
		"another encoder",
		r#"let a = toml::to_string(&v).unwrap_or_default();"#,
	),
	(
		"the byte vector encoder",
		r#"let a = serde_json::to_vec(&v).unwrap_or_default();"#,
	),
	(
		"the encoder that writes into a writer",
		r#"let a = serde_json::to_writer(&mut out, &v).unwrap_or_default();"#,
	),
	(
		"the encoder imported by name",
		r#"use serde_json::to_string;
let a = to_string(&v).unwrap_or_default();"#,
	),
	(
		"the result kept in a name and thrown away later",
		r#"let a = serde_json::to_string(&v);
let b = a.unwrap_or_default();"#,
	),
];

/// The shapes the rule must leave alone, and why each one is not the defect
/// the rule is about.
const ALLOWED: &[(&str, &str)] = &[
	(
		"the error is propagated",
		r#"let a = serde_json::to_string(&v)?;"#,
	),
	(
		"the error is propagated with a message",
		r#"let a = serde_json::to_string(&v).map_err(|e| e.to_string())?;"#,
	),
	(
		"a comment that names the old form",
		r#"// used to call serde_json::to_string(&v).unwrap_or_default()"#,
	),
	(
		"a block comment that names the old form",
		r#"/* serde_json::to_string(&v).unwrap_or_default() */"#,
	),
	(
		"a string literal that holds the old form",
		r#"let note = "serde_json::to_string(&v).unwrap_or_default()";"#,
	),
	(
		"a default that belongs to something else",
		r#"let a = parts.next().unwrap_or_default().to_string();"#,
	),
	(
		"`to_value` answers with a value, not with text: its default is a null, \
		 not an empty document",
		r#"let a = serde_json::to_value(&v).unwrap_or_default();"#,
	),
	(
		"`to_string` on a value is `Display`, not a serialisation",
		r#"let a = name.to_string();"#,
	),
	(
		"a fallback that is not empty",
		r#"let a = serde_json::to_string(&v).unwrap_or(FALLBACK);"#,
	),
];

#[test]
fn every_forbidden_shape_is_found() {
	for (name, code) in FORBIDDEN {
		let found = detector::findings(&lexer::tokenize(code));
		assert!(
			!found.is_empty(),
			"the shape `{name}` must be found, and it was not:\n{code}"
		);
	}
}

#[test]
fn no_allowed_shape_is_found() {
	for (name, code) in ALLOWED {
		let found = detector::findings(&lexer::tokenize(code));
		assert!(
			found.is_empty(),
			"the shape `{name}` must not be found, and it was:\n{code}\n{found:?}"
		);
	}
}

/// The control: a shape the detector is written against is found, so a
/// detector that matched nothing would fail here rather than pass the tests
/// above by finding nothing at all.
#[test]
fn the_detector_finds_the_shape_it_is_written_against() {
	let found = detector::findings(&lexer::tokenize(
		r#"let a = serde_json::to_string(&v).unwrap_or_default();"#,
	));
	assert_eq!(found.len(), 1, "{found:?}");
	assert_eq!(found[0].shape, "unwrap_or_default");
	assert_eq!(found[0].line, 1);
}
