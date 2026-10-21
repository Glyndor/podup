use super::*;

#[test]
fn build_per_network_options_seeds_service_name_alias() {
	// With no explicit config, the service name is still registered as an
	// alias so siblings can reach the service by name.
	let opts = build_per_network_options("web", None, None);
	assert_eq!(opts.aliases, vec!["web".to_string()]);
	assert!(opts.static_ips.is_empty());
}

#[test]
fn build_per_network_options_empty_service_name_adds_no_alias() {
	let opts = build_per_network_options("", None, None);
	assert!(opts.aliases.is_empty());
}

#[test]
fn build_per_network_options_with_aliases() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		aliases: Some(vec!["api".to_string()]),
		..Default::default()
	};
	// The service name is prepended ahead of any explicit aliases.
	let opts = build_per_network_options("web", Some(&cfg), None);
	assert_eq!(opts.aliases, vec!["web".to_string(), "api".to_string()]);
}

#[test]
fn build_per_network_options_does_not_duplicate_service_name() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		aliases: Some(vec!["web".to_string(), "api".to_string()]),
		..Default::default()
	};
	// An explicit alias equal to the service name is not duplicated.
	let opts = build_per_network_options("web", Some(&cfg), None);
	assert_eq!(opts.aliases, vec!["web".to_string(), "api".to_string()]);
}

#[test]
fn build_per_network_options_with_ipv4() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		ipv4_address: Some("10.0.0.5".to_string()),
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None);
	assert!(opts.static_ips.contains(&"10.0.0.5".to_string()));
}

#[test]
fn fallback_mac_applied_when_no_config() {
	let opts = build_per_network_options("web", None, Some("02:42:ac:11:00:02"));
	assert_eq!(opts.static_mac.as_deref(), Some("02:42:ac:11:00:02"));
}

#[test]
fn per_network_interface_name_forwarded() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		interface_name: Some("eth1".into()),
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None);
	assert_eq!(opts.interface_name.as_deref(), Some("eth1"));
}

#[test]
fn per_network_mac_takes_precedence_over_fallback() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		mac_address: Some("aa:bb:cc:dd:ee:ff".to_string()),
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), Some("02:42:ac:11:00:03"));
	assert_eq!(opts.static_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
}

#[test]
fn per_network_ipv6_and_link_local_become_static_ips() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		ipv4_address: Some("10.0.0.5".to_string()),
		ipv6_address: Some("2001:db8::5".to_string()),
		link_local_ips: vec!["169.254.0.1".to_string()],
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None);
	// All three address forms are folded into the static-IP list.
	assert!(opts.static_ips.contains(&"10.0.0.5".to_string()));
	assert!(opts.static_ips.contains(&"2001:db8::5".to_string()));
	assert!(opts.static_ips.contains(&"169.254.0.1".to_string()));
}

#[test]
fn per_network_priority_folds_into_driver_opts() {
	use crate::compose::types::ServiceNetworkConfig;
	let mut driver_opts = std::collections::HashMap::new();
	driver_opts.insert("mtu".to_string(), "1400".to_string());
	let cfg = ServiceNetworkConfig {
		priority: Some(100),
		driver_opts,
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None);
	let opts = opts.driver_opts.expect("driver opts present");
	// Compose `priority` is consumed by Podman as a driver option, alongside the
	// explicit driver_opts.
	assert_eq!(opts.get("priority").map(String::as_str), Some("100"));
	assert_eq!(opts.get("mtu").map(String::as_str), Some("1400"));
}
