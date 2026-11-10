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

/// A secret or config with a source is staged from it whatever its `driver:`
/// says (measured: the file's content is mounted); only the plugin is ignored.
/// The warning must not say it will not be staged.
#[test]
fn a_driver_on_a_definition_with_a_source_does_not_claim_nothing_is_staged() {
	for (kind, yaml) in [
		("secret", "services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    file: ./tok.txt\n    driver: vault\n"),
		("config", "services:\n  web:\n    image: nginx\n    configs: [c]\nconfigs:\n  c:\n    content: x\n    driver: vault\n"),
	] {
		let msgs = diagnostics_for(yaml);
		let msg = msgs.iter().find(|m| m.contains("driver")).unwrap_or_else(|| panic!("{kind}: {msgs:?}"));
		assert!(msg.contains("is staged from its"), "{msg}");
		assert!(!msg.contains("will not be staged"), "{msg}");
	}
}

#[test]
fn a_driver_on_a_definition_with_no_source_says_there_is_nothing_to_stage() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    secrets: [tok]\nsecrets:\n  tok:\n    driver: vault\n",
	);
	let msg = msgs
		.iter()
		.find(|m| m.contains("driver"))
		.expect("a warning");
	assert!(msg.contains("nothing to stage"), "{msg}");
}
