use super::*;

#[test]
fn warns_on_unknown_top_level_key_but_not_x_extension() {
	let msgs = diagnostics_for(
		"x-anchors: ok\nservies:\n  typo: 1\nservices:\n  web:\n    image: nginx\n",
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("unknown top-level key 'servies'")),
		"got: {msgs:?}"
	);
	assert!(!msgs.iter().any(|m| m.contains("x-anchors")));
}

#[test]
fn warns_on_unknown_service_key_but_not_x_extension() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    enviroment:\n      A: 1\n    x-meta: ok\n",
	);
	assert!(msgs.iter().any(|m| m.contains("unknown key 'enviroment'")));
	assert!(!msgs.iter().any(|m| m.contains("x-meta")));
}

#[test]
fn warns_on_unknown_develop_watch_key() {
	// An unrecognized key inside a `develop.watch[*]` rule is surfaced with the
	// indexed context, but an `x-` extension key on the same rule is left alone.
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    develop:\n      watch:\n        - path: ./src\n          action: sync\n          target: /app\n          bogus_key: 1\n          x-note: ok\n",
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("develop.watch[0]") && m.contains("bogus_key")),
		"got: {msgs:?}"
	);
	assert!(!msgs.iter().any(|m| m.contains("x-note")));
}

#[test]
fn nested_x_extension_key_is_not_flagged() {
	// An `x-` key inside a modeled sub-object (here, healthcheck) is a valid
	// extension and must not produce an "unknown key" warning.
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n      x-custom: ok\n",
	);
	assert!(
		!msgs.iter().any(|m| m.contains("x-custom")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_unknown_key_in_healthcheck() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n      retires: 3\n",
		);
	assert!(
		msgs.iter()
			.any(|m| m.contains("healthcheck") && m.contains("retires")),
		"got: {msgs:?}"
	);
}

#[test]
fn warns_on_unknown_key_in_network_and_ipam() {
	let msgs = diagnostics_for(
			"services:\n  web:\n    image: nginx\nnetworks:\n  net:\n    drivr: bridge\n    ipam:\n      confg: []\n",
		);
	assert!(msgs
		.iter()
		.any(|m| m.contains("network 'net'") && m.contains("drivr")));
	assert!(msgs
		.iter()
		.any(|m| m.contains("ipam") && m.contains("confg")));
}

#[test]
fn warns_on_unknown_key_in_volume() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\nvolumes:\n  data:\n    externl: true\n",
	);
	assert!(msgs
		.iter()
		.any(|m| m.contains("volume 'data'") && m.contains("externl")));
}

#[test]
fn warns_on_typo_inside_provider_and_models() {
	let msgs = diagnostics_for(
			"services:\n  db:\n    provider:\n      type: cloud\n      optoins: {}\nmodels:\n  llm:\n    modle: ai/model\n",
		);
	assert!(
		msgs.iter()
			.any(|m| m.contains("provider") && m.contains("optoins")),
		"got: {msgs:?}"
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("model 'llm'") && m.contains("modle")),
		"got: {msgs:?}"
	);
}
