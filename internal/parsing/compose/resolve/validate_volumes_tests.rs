use crate::error::{ComposeError, Result};
use crate::parse_str;

fn validate_volume(attrs: &str) -> Result<()> {
	let yaml = format!(
		"services:\n  web:\n    image: alpine\nvolumes:\n  data:\n    external: true\n{attrs}"
	);
	let mut file = parse_str(&yaml).unwrap();
	crate::compose::normalize_default_network(&mut file);
	super::super::validate::validate(&file)
}

fn message(attrs: &str) -> String {
	match validate_volume(attrs).unwrap_err() {
		ComposeError::Unsupported(text) => text,
		other => panic!("expected ComposeError::Unsupported, got {other:?}"),
	}
}

/// The engine never creates an external volume, so these were dropped without
/// a word; Docker Compose refuses the file.
#[test]
fn an_external_volume_with_a_driver_is_refused_naming_the_field() {
	let text = message("    driver: local\n");
	assert!(
		text.contains("volume 'data'") && text.contains("'driver'"),
		"{text}"
	);
}

#[test]
fn an_external_volume_with_driver_opts_is_refused() {
	let text = message("    driver_opts:\n      type: nfs\n");
	assert!(text.contains("'driver_opts'"), "{text}");
}

#[test]
fn an_external_volume_with_labels_is_refused() {
	let text = message("    labels:\n      team: a\n");
	assert!(text.contains("'labels'"), "{text}");
}

/// `name:` says which existing volume to use, and a volume that is not external
/// may carry every attribute.
#[test]
fn a_name_on_an_external_volume_and_attributes_on_a_created_one_are_fine() {
	validate_volume("    name: shared\n").expect("external with a name");
	let yaml = "services:\n  web:\n    image: alpine\nvolumes:\n  data:\n    driver: local\n    labels:\n      team: a\n";
	let file = parse_str(yaml).unwrap();
	super::super::validate::validate(&file).expect("a created volume keeps its attributes");
}
