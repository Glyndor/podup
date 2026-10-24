use super::*;

/// An init step is a container the operator wrote, so the diagnostics that
/// walk a service walk it too: a typo inside a step is a typo, and it is
/// reported against the step that carries it.
#[test]
fn warns_on_unknown_key_inside_an_init_step() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    pre_start:\n      - command: init\n        enviroment:\n          A: 1\n",
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("web/pre_start[1]") && m.contains("unknown key 'enviroment'")),
		"got: {msgs:?}"
	);
}

/// The step is reported on its own: the service's own keys are not repeated
/// under the step's name.
#[test]
fn does_not_repeat_the_service_keys_under_the_step() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    enviroment:\n      A: 1\n    pre_start:\n      - command: init\n",
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("service 'web'") && m.contains("unknown key 'enviroment'")),
		"got: {msgs:?}"
	);
	assert!(
		!msgs.iter().any(|m| m.contains("web/pre_start[1]")),
		"the step must not repeat the service's own keys: {msgs:?}"
	);
}

/// A step that publishes a port on every interface is reported like any
/// other container that does.
#[test]
fn warns_on_a_port_published_on_every_interface_by_a_step() {
	let msgs = diagnostics_for(
		"services:\n  web:\n    image: nginx\n    pre_start:\n      - ports: ['5432:5432']\n",
	);
	assert!(
		msgs.iter()
			.any(|m| m.contains("web/pre_start[1]") && m.contains("published on every interface")),
		"got: {msgs:?}"
	);
}
