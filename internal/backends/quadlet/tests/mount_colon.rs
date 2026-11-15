//! A long-form mount path with a colon would be split in the wrong place by
//! `Volume=source:target`, so the export refuses it while `up` keeps it.

use crate::parse_str;
use crate::quadlet::validate_for_quadlet;

fn mount(source: &str, target: &str) -> String {
	format!(
		"services:\n  web:\n    image: nginx\n    volumes:\n      - type: bind\n        source: '{source}'\n        target: '{target}'\n"
	)
}

#[test]
fn a_colon_in_the_source_or_target_is_refused() {
	for yaml in [mount("/srv/a:b", "/data"), mount("/srv/a", "/data:x")] {
		let err = validate_for_quadlet(&parse_str(&yaml).unwrap())
			.expect_err("a colon in a mount path must fail validation");
		assert!(err.to_string().contains("colon"), "{err}");
	}
}

#[test]
fn a_plain_bind_is_accepted() {
	validate_for_quadlet(&parse_str(&mount("/srv/a", "/data")).unwrap()).unwrap();
}
