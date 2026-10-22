use super::*;

#[test]
fn file_secret_produces_no_diagnostics() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    file: ./tok.txt\n",
		);
	assert!(msgs.is_empty(), "unexpected diagnostics: {msgs:?}");
}

#[test]
fn warns_on_non_external_secret_driver() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    driver: vault\n",
		);
	assert!(
		msgs.iter().any(|m| m.contains("secret 'tok': driver")),
		"got: {msgs:?}"
	);
}

#[test]
fn external_secret_driver_produces_no_diagnostic() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n    driver: vault\n",
		);
	assert!(
		!msgs.iter().any(|m| m.contains("driver")),
		"unexpected: {msgs:?}"
	);
}

#[test]
fn warns_on_secret_template_driver() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    template_driver: golang\n",
		);
	assert!(
		msgs.iter()
			.any(|m| m.contains("secret 'tok': template_driver")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_non_external_config_driver_and_template_driver() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\nconfigs:\n  conf:\n    driver: vault\n    template_driver: golang\n",
		);
	assert!(
		msgs.iter().any(|m| m.contains("config 'conf': driver")),
		"got: {msgs:?}"
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("config 'conf': template_driver")),
		"got: {msgs:?}"
	);
}

#[test]
fn external_config_driver_produces_no_diagnostic() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\nconfigs:\n  conf:\n    external: true\n    driver: vault\n",
		);
	assert!(
		!msgs.iter().any(|m| m.contains("driver")),
		"unexpected: {msgs:?}"
	);
}
