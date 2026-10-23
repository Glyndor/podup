use super::*;

/// Docker isolates bridge networks from each other and podman does not, so a
/// podup project used to let one service reach unpublished ports on another
/// network. Measured on both engines; see `apply_default_isolation`.
#[test]
fn a_bridge_network_is_isolated_by_default() {
	let mut opts = HashMap::new();
	apply_default_isolation("bridge", &mut opts);
	assert_eq!(opts.get("isolate").map(String::as_str), Some("true"));
}

/// The option is netavark's bridge plugin. Setting it on a driver that does
/// not read it would be noise at best and a create failure at worst.
#[test]
fn only_bridge_networks_get_the_option() {
	for driver in ["macvlan", "ipvlan", "host"] {
		let mut opts = HashMap::new();
		apply_default_isolation(driver, &mut opts);
		assert!(opts.is_empty(), "{driver} should not be given isolate");
	}
}

/// The escape hatch for a project that deliberately spans networks. A default
/// that cannot be turned off is not a default, it is a policy.
#[test]
fn an_explicit_driver_opt_wins_in_both_directions() {
	let mut off = HashMap::from([("isolate".to_string(), "false".to_string())]);
	apply_default_isolation("bridge", &mut off);
	assert_eq!(off.get("isolate").map(String::as_str), Some("false"));

	let mut strict = HashMap::from([("isolate".to_string(), "strict".to_string())]);
	apply_default_isolation("bridge", &mut strict);
	assert_eq!(strict.get("isolate").map(String::as_str), Some("strict"));
}

/// Whatever else the compose file asked for has to survive.
#[test]
fn other_driver_opts_are_left_alone() {
	let mut opts = HashMap::from([("mtu".to_string(), "9000".to_string())]);
	apply_default_isolation("bridge", &mut opts);
	assert_eq!(opts.get("mtu").map(String::as_str), Some("9000"));
	assert_eq!(opts.get("isolate").map(String::as_str), Some("true"));
}
