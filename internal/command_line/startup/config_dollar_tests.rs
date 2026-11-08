use super::*;
use crate::cli::ConfigFormat;
use crate::startup::{render_config_to, ConfigOutput};
use std::path::Path;

fn render(yaml: &str, format: &ConfigFormat, no_interpolate: bool) -> String {
	// `--no-interpolate` loads the file without interpolating it.
	let file = if no_interpolate {
		podup::parse_str_raw(yaml).unwrap()
	} else {
		podup::parse_str(yaml).unwrap()
	};
	let mut out = Vec::new();
	render_config_to(
		&mut out,
		&file,
		format,
		&ConfigOutput {
			no_interpolate,
			..Default::default()
		},
		"proj",
		Path::new("/proj"),
	)
	.unwrap();
	String::from_utf8(out).unwrap()
}

const SOURCE: &str = "services:\n  web:\n    image: nginx\n    environment:\n      LIT: \"cost $$5\"\n    command: [\"echo\", \"$$X\"]\n    labels:\n      l: \"$$HOME\"\n";

#[test]
fn a_literal_dollar_in_a_value_is_printed_doubled_in_yaml() {
	let text = render(SOURCE, &ConfigFormat::Yaml, false);
	assert!(text.contains("LIT: cost $$5"), "{text}");
	assert!(text.contains("- $$X"), "{text}");
	assert!(text.contains("l: $$HOME"), "{text}");
}

#[test]
fn and_in_json() {
	let text = render(SOURCE, &ConfigFormat::Json, false);
	assert!(text.contains("\"LIT\": \"cost $$5\""), "{text}");
	assert!(text.contains("\"$$X\""), "{text}");
}

#[test]
fn the_output_reads_back_as_the_same_model() {
	// What the doubling is for: `config | podup -f - ...` interpolates the text a
	// second time, and a bare `$5` or `$X` would be taken for a reference.
	let printed = render(SOURCE, &ConfigFormat::Yaml, false);
	let first = podup::parse_str(SOURCE).unwrap();
	let again = podup::parse_str(&printed).unwrap();
	let value = |f: &podup::compose::types::ComposeFile| {
		serde_json::to_value(&f.services["web"].environment).unwrap()
	};
	assert_eq!(value(&first), value(&again));
	assert_eq!(
		first.services["web"].labels.to_map(),
		again.services["web"].labels.to_map()
	);
}

#[test]
fn no_interpolate_prints_the_text_as_it_was_written() {
	let text = render(SOURCE, &ConfigFormat::Yaml, true);
	assert!(text.contains("LIT: cost $$5"), "{text}");
	assert!(!text.contains("$$$$"), "escaped twice: {text}");
}

#[test]
fn a_value_without_a_dollar_and_the_keys_are_left_alone() {
	let mut v = serde_json::json!({"a$b": "plain", "list": ["x", "y$"]});
	escape_json(&mut v);
	assert_eq!(v, serde_json::json!({"a$b": "plain", "list": ["x", "y$$"]}));
	let mut y: serde_yaml::Value = serde_yaml::from_str("k: v\nn: 3\nb: true\ns: a$b\n").unwrap();
	escape_yaml(&mut y);
	assert_eq!(y["s"], serde_yaml::Value::String("a$$b".into()));
	assert_eq!(y["n"], serde_yaml::Value::Number(3.into()));
}
