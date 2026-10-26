//! Text preservation, preflight checks and directive handling for masks.

use crate::compose::types::Service;
use crate::compose::{parse_files_with_env_files_interp, parse_str_raw};

#[test]
fn umask_config_no_interpolate_refuses_placeholder() {
	let dir = tempfile::tempdir().unwrap();
	let path = dir.path().join("compose.yml");
	std::fs::write(
		&path,
		"services:\n  web:\n    image: alpine\n    umask: '${MASK:-0022}'\n",
	)
	.unwrap();
	let error = parse_files_with_env_files_interp(&[path], &[], false)
		.unwrap_err()
		.to_string();
	assert!(error.contains("web") && error.contains("quote"), "{error}");
	assert!(error.contains("variable did not resolve"), "{error}");
}

#[test]
fn umask_interpolation_keeps_octal_text() {
	let vars = std::collections::HashMap::new();
	for text in ["0022", "022", "0"] {
		let file = crate::compose::merge::deserialize_with_merge_interp(
			&format!("services:\n  web:\n    umask: '${{MASK:-{text}}}'\n"),
			Some(&vars),
		)
		.unwrap();
		assert_eq!(file.services["web"].umask.as_deref(), Some(text));
	}
	let error = crate::compose::merge::deserialize_with_merge_interp(
		"services:\n  worker:\n    umask: '${MASK:-10000}'\n",
		Some(&vars),
	)
	.unwrap_err()
	.to_string();
	assert!(
		error.contains("worker") && error.contains("umask"),
		"{error}"
	);
}

#[test]
fn umask_preflight_rejects_invalid_model_text() {
	for text in ["10000", "00000", "88", "", "${MASK:-0022}", " 22", "0x16"] {
		let service = Service {
			umask: Some(text.into()),
			..Default::default()
		};
		let error = crate::libpod::validate::pre_validate_spec("worker", &service)
			.unwrap_err()
			.to_string();
		assert!(
			error.contains("worker") && error.contains("umask"),
			"{error}"
		);
	}
	for text in ["0", "22", "0022", "7777"] {
		let service = Service {
			umask: Some(text.into()),
			..Default::default()
		};
		crate::libpod::validate::pre_validate_spec("worker", &service).unwrap();
	}
}

#[test]
fn umask_serialization_normalizes_only_valid_text() {
	for (text, output) in [
		("0", "0000"),
		("22", "0022"),
		("022", "0022"),
		("7777", "7777"),
	] {
		let service = Service {
			umask: Some(text.into()),
			..Default::default()
		};
		assert_eq!(serde_json::to_value(&service).unwrap()["umask"], output);
		{
			let yaml = serde_yaml::to_string(&service).unwrap();
			let restored: Service = serde_yaml::from_str(&yaml).unwrap();
			assert_eq!(restored.umask.as_deref(), Some(output));
		}
	}
}

#[test]
fn umask_omission_preserves_hash_input() {
	let file = parse_str_raw("services:\n  web:\n    image: alpine\n").unwrap();
	let json = serde_json::to_value(&file.services["web"]).unwrap();
	assert!(json.get("umask").is_none());
	assert!(json.get("umask_merge_tag").is_none());
	let mut previous = json.clone();
	previous.as_object_mut().unwrap().remove("umask");
	assert_eq!(
		serde_json::to_vec(&json).unwrap(),
		serde_json::to_vec(&previous).unwrap()
	);
}

#[test]
fn umask_unresolved_values_are_refused() {
	let vars = std::collections::HashMap::new();
	for text in ["$$", "$${MASK}", "${MASK:-bad$}"] {
		let error = crate::compose::merge::deserialize_with_merge_interp(
			&format!("services:\n  worker:\n    umask: '{text}'\n"),
			Some(&vars),
		)
		.unwrap_err()
		.to_string();
		assert!(
			error.contains("worker") && error.contains("quote"),
			"{error}"
		);
		assert!(error.contains("variable did not resolve"), "{error}");
	}
}

#[test]
fn umask_normal_interpolation_retains_numeric_rules() {
	let vars = std::collections::HashMap::new();
	for text in ["22", "7777", "true"] {
		let error = crate::compose::merge::deserialize_with_merge_interp(
			&format!("services:\n  worker:\n    umask: '${{MASK:-{text}}}'\n"),
			Some(&vars),
		)
		.unwrap_err()
		.to_string();
		assert!(
			error.contains("worker") && error.contains("quote"),
			"{error}"
		);
	}
}

#[test]
fn umask_serialization_refuses_invalid_model_text() {
	for text in ["${MASK}", "10000", "0022 --privileged"] {
		let service = Service {
			umask: Some(text.into()),
			..Default::default()
		};
		let error = serde_json::to_value(service).unwrap_err().to_string();
		assert!(error.contains("quote"), "{error}");
	}
}
