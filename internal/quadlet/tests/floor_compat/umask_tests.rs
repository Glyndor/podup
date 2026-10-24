//! Initial process masks travel through the floor-compatible CLI passthrough.

use crate::compose::parse_str;
use crate::quadlet::{generate_at, min_podman};

#[test]
fn umask_quadlet_uses_floor_passthrough() {
	for (value, flag) in [
		("'0022'", Some("0022")),
		("0", Some("0000")),
		("null", None),
	] {
		let file = parse_str(&format!(
			"services:\n  web:\n    image: alpine\n    umask: {value}\n"
		))
		.unwrap();
		let out = generate_at(&file, "demo", std::path::Path::new("/srv"));
		let unit = out
			.units
			.iter()
			.find(|u| u.filename == "demo-web.container")
			.unwrap();
		assert!(!unit.contents.contains("Umask="), "{}", unit.contents);
		if let Some(mask) = flag {
			assert!(
				unit.contents
					.contains(&format!("PodmanArgs=--umask={mask}\n")),
				"{}",
				unit.contents
			);
		} else {
			assert!(!unit.contents.contains("--umask="), "{}", unit.contents);
		}
		for key in min_podman::keys_in_unit(&unit.contents) {
			assert!(
				min_podman::is_at_floor(min_podman::UnitType::Container, &key),
				"{key}"
			);
		}
		assert!(
			out.warnings.iter().all(|w| !w.contains("umask")),
			"{:?}",
			out.warnings
		);
	}
}

#[test]
fn umask_quadlet_refuses_invalid_final_values() {
	for text in ["0022 --privileged", "${MASK}", "$", "10000"] {
		let parsed_error = crate::compose::parse_str_raw(&format!(
			"services:\n  worker:\n    image: alpine\n    umask: '{text}'\n"
		))
		.unwrap_err()
		.to_string();
		assert!(
			parsed_error.contains("worker") && parsed_error.contains("umask"),
			"{parsed_error}"
		);
		let mut file = parse_str("services:\n  worker:\n    image: alpine\n").unwrap();
		file.services["worker"].umask = Some(text.into());
		let error = crate::quadlet::validate_for_quadlet(&file)
			.unwrap_err()
			.to_string();
		assert!(
			error.contains("worker") && error.contains("umask"),
			"{error}"
		);
		assert!(error.contains("quote"), "{error}");
	}
}

#[test]
fn umask_quadlet_never_emits_invalid_arguments() {
	let mut file = parse_str("services:\n  worker:\n    image: alpine\n").unwrap();
	file.services["worker"].umask = Some("0022 --privileged".into());
	let out = generate_at(&file, "demo", std::path::Path::new("/srv"));
	assert!(out
		.units
		.iter()
		.all(|unit| !unit.contents.contains("--umask=") && !unit.contents.contains("--privileged")));
}
