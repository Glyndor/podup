//! Quadlet reads an image name ending in `.build` or `.image` as a unit
//! reference, so the export refuses it rather than write a unit that cannot start.

use crate::parse_str;
use crate::quadlet::validate_for_quadlet;

fn image(name: &str) -> String {
	format!("services:\n  web:\n    image: {name}\n")
}

#[test]
fn an_image_ending_in_build_or_image_is_refused() {
	for name in ["myimg.build", "registry.example/app.image", "other.image"] {
		let err = validate_for_quadlet(&parse_str(&image(name)).unwrap())
			.expect_err("a unit-like image name must fail validation");
		let text = err.to_string();
		assert!(text.contains(".build or .image"), "{name}: {text}");
	}
}

#[test]
fn an_ordinary_image_name_is_accepted() {
	for name in [
		"alpine",
		"docker.io/library/alpine:3",
		"app.builder",
		"x/image:1",
	] {
		validate_for_quadlet(&parse_str(&image(name)).unwrap())
			.unwrap_or_else(|e| panic!("{name} was refused: {e}"));
	}
}

#[test]
fn a_service_that_builds_may_carry_such_a_name() {
	let yaml = "services:\n  web:\n    image: local.build\n    build: .\n";
	validate_for_quadlet(&parse_str(yaml).unwrap()).expect("a built image is not a unit reference");
}
