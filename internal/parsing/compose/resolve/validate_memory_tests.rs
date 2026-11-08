use crate::error::{ComposeError, Result};
use crate::parse_str;

fn validate_str(yaml: &str) -> Result<()> {
	let mut file = parse_str(yaml).unwrap();
	crate::compose::normalize_default_network(&mut file);
	super::super::validate::validate(&file)
}

fn service_with(line: &str) -> String {
	format!("services:\n  web:\n    image: alpine\n    {line}\n")
}

fn message(line: &str) -> String {
	match validate_str(&service_with(line)).unwrap_err() {
		ComposeError::Invalid(text) => text,
		other => panic!("expected ComposeError::Invalid, got {other:?}"),
	}
}

#[test]
fn impossible_mem_limit_names_service_and_key() {
	assert_eq!(
		message("mem_limit: \"1e100\""),
		"service 'web': mem_limit \"1e100\" is not a valid memory amount"
	);
}

#[test]
fn garbage_mem_limit_is_an_error() {
	assert!(message("mem_limit: lots").contains("mem_limit \"lots\""));
}

#[test]
fn negative_mem_limit_is_an_error() {
	assert!(message("mem_limit: -5m").contains("mem_limit \"-5m\""));
	assert!(message("mem_limit: \"-2\"").contains("mem_limit \"-2\""));
}

#[test]
fn overflowing_mem_limit_is_an_error() {
	assert!(message("mem_limit: 99999999999t").contains("not a valid memory amount"));
}

#[test]
fn empty_amount_is_unset_and_renders_no_limit() {
	for key in ["mem_limit", "memswap_limit", "mem_reservation", "shm_size"] {
		validate_str(&service_with(&format!("{key}: \"\""))).unwrap();
	}
	let file = parse_str(&service_with("mem_limit: \"\"")).unwrap();
	let service = &file.services["web"];
	assert_eq!(crate::effective_memory_limit(service), None);
	let out = crate::quadlet::generate_at(&file, "p", std::path::Path::new("/srv/app"));
	assert!(
		out.units
			.iter()
			.all(|unit| !unit.contents.contains("--memory")),
		"an empty mem_limit must render no limit"
	);
}

#[test]
fn sibling_keys_are_checked_with_their_own_name() {
	for key in ["memswap_limit", "mem_reservation", "shm_size"] {
		let text = message(&format!("{key}: nonsense"));
		assert_eq!(
			text,
			format!("service 'web': {key} \"nonsense\" is not a valid memory amount")
		);
	}
}

#[test]
fn deploy_memory_keys_are_checked() {
	for (block, key) in [
		("limits", "deploy.resources.limits.memory"),
		("reservations", "deploy.resources.reservations.memory"),
	] {
		let yaml = format!(
			"services:\n  web:\n    image: alpine\n    deploy:\n      resources:\n        {block}:\n          memory: \"1e100\"\n"
		);
		let text = match validate_str(&yaml).unwrap_err() {
			ComposeError::Invalid(text) => text,
			other => panic!("expected Invalid, got {other:?}"),
		};
		assert_eq!(
			text,
			format!("service 'web': {key} \"1e100\" is not a valid memory amount")
		);
	}
}

#[test]
fn a_job_is_checked_in_its_own_words() {
	let yaml = "jobs:\n  nightly:\n    image: alpine\n    mem_limit: 1e100\n    triggers: {manual: true}\n";
	let text = match validate_str(yaml).unwrap_err() {
		ComposeError::Invalid(text) => text,
		other => panic!("expected Invalid, got {other:?}"),
	};
	assert!(text.contains("mem_limit \"1e100\""), "{text}");
}

#[test]
fn valid_amounts_still_pass() {
	for value in [
		"1024", "512b", "512m", "512M", "1g", "1.5g", "64mb", "2t", "0", "-1",
	] {
		validate_str(&service_with(&format!("mem_limit: \"{value}\"")))
			.unwrap_or_else(|e| panic!("{value}: {e}"));
	}
	validate_str(&service_with("memswap_limit: -1")).unwrap();
	validate_str(&service_with("shm_size: 64m")).unwrap();
	validate_str(&service_with("mem_reservation: 256m")).unwrap();
}

#[test]
fn deploy_valid_memory_passes() {
	let yaml = "services:\n  web:\n    image: alpine\n    deploy:\n      resources:\n        limits:\n          memory: 512M\n        reservations:\n          memory: 128M\n";
	validate_str(yaml).unwrap();
}
