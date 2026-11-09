use crate::cli::ConfigFormat;
use crate::startup::{render_config_to, ConfigOutput};
use std::path::Path;

const SOURCE: &str = "services:\n  s:\n    image: busybox\n    volumes: [\"a:/a\", \"b:/b\", \"c:/c\", \"d:/d\"]\nvolumes:\n  a: {}\n  b:\n  c: {name: custom}\n  d: {external: true}\n";

fn render(yaml: &str) -> String {
	let file = podup::parse_str(yaml).unwrap();
	let mut out = Vec::new();
	render_config_to(
		&mut out,
		&file,
		&ConfigFormat::Yaml,
		&ConfigOutput::default(),
		"proj",
		Path::new("/proj"),
	)
	.unwrap();
	String::from_utf8(out).unwrap()
}

#[test]
fn a_bare_volume_is_printed_with_its_resolved_name() {
	let out = render(SOURCE);
	let value: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
	assert_eq!(value["volumes"]["a"]["name"], "proj_a", "{out}");
	assert_eq!(value["volumes"]["b"]["name"], "proj_b", "{out}");
}

#[test]
fn a_named_or_external_volume_keeps_what_it_says() {
	let out = render(SOURCE);
	let value: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
	assert_eq!(value["volumes"]["c"]["name"], "custom", "{out}");
	assert!(value["volumes"]["d"].get("name").is_none(), "{out}");
	assert_eq!(value["volumes"]["d"]["external"], true, "{out}");
}

/// The printed file declares every volume it mounts, so it reads back and
/// prints the same again.
#[test]
fn the_output_declares_every_volume_it_mounts_and_reads_back() {
	let first = render(SOURCE);
	let value: serde_yaml::Value = serde_yaml::from_str(&first).unwrap();
	for key in ["a", "b", "c", "d"] {
		assert!(
			value["volumes"].get(key).is_some(),
			"{key} missing:\n{first}"
		);
	}
	assert_eq!(first, render(&first));
}
