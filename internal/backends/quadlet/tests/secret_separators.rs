//! A secret field with a separator would move the split of `Secret=`, and the unit
//! writer drops it silently, so the export refuses it instead.

use crate::parse_str;
use crate::quadlet::validate_for_quadlet;

fn with_target(target: &str) -> String {
	format!(
		"secrets:\n  s1:\n    file: ./s\nservices:\n  web:\n    image: x\n    secrets:\n      - source: s1\n        target: \"{target}\"\n"
	)
}

#[test]
fn a_comma_or_equals_in_a_secret_target_is_refused() {
	for target in ["a,b", "a=b", "tok,uid=0"] {
		let err = validate_for_quadlet(&parse_str(&with_target(target)).unwrap())
			.expect_err("a separator in a secret target must fail validation");
		assert!(err.to_string().contains("secret target"), "{target}: {err}");
	}
}

#[test]
fn an_ordinary_secret_target_is_accepted() {
	for target in ["token", "db-password", "/run/secrets/key.pem"] {
		validate_for_quadlet(&parse_str(&with_target(target)).unwrap())
			.unwrap_or_else(|e| panic!("{target} was refused: {e}"));
	}
}

#[test]
fn a_separator_in_a_short_form_secret_name_is_refused() {
	let yaml = "secrets:\n  \"a,b\":\n    file: ./s\nservices:\n  web:\n    image: x\n    secrets: [\"a,b\"]\n";
	let err = validate_for_quadlet(&parse_str(yaml).unwrap())
		.expect_err("a separator in a secret name must fail validation");
	assert!(err.to_string().contains("secret source"), "{err}");
}
