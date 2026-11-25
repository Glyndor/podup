use super::*;

#[test]
fn build_per_network_options_seeds_service_name_alias() {
	// With no explicit config, the service name is still registered as an
	// alias so siblings can reach the service by name.
	let opts = build_per_network_options("web", None, None, &[]);
	assert_eq!(opts.aliases, vec!["web".to_string()]);
	assert!(opts.static_ips.is_empty());
}

#[test]
fn build_per_network_options_empty_service_name_adds_no_alias() {
	let opts = build_per_network_options("", None, None, &[]);
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
	let opts = build_per_network_options("web", Some(&cfg), None, &[]);
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
	let opts = build_per_network_options("web", Some(&cfg), None, &[]);
	assert_eq!(opts.aliases, vec!["web".to_string(), "api".to_string()]);
}

#[test]
fn build_per_network_options_with_ipv4() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		ipv4_address: Some("10.0.0.5".to_string()),
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None, &[]);
	assert!(opts.static_ips.contains(&"10.0.0.5".to_string()));
}

#[test]
fn fallback_mac_applied_when_no_config() {
	let opts = build_per_network_options("web", None, Some("02:42:ac:11:00:02"), &[]);
	assert_eq!(opts.static_mac.as_deref(), Some("02:42:ac:11:00:02"));
}

#[test]
fn per_network_interface_name_forwarded() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		interface_name: Some("eth1".into()),
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None, &[]);
	assert_eq!(opts.interface_name.as_deref(), Some("eth1"));
}

#[test]
fn per_network_mac_takes_precedence_over_fallback() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		mac_address: Some("aa:bb:cc:dd:ee:ff".to_string()),
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), Some("02:42:ac:11:00:03"), &[]);
	assert_eq!(opts.static_mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
}

#[test]
fn per_network_ipv4_and_ipv6_become_static_ips_and_link_local_does_not() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		ipv4_address: Some("10.0.0.5".to_string()),
		ipv6_address: Some("2001:db8::5".to_string()),
		link_local_ips: vec!["169.254.0.1".to_string()],
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None, &[]);
	// The two addresses of the container are static IPs. A link-local address
	// is an extra one on the interface, which Podman cannot add; as a static IP
	// it fails at start when it lies outside the subnet.
	assert_eq!(
		opts.static_ips,
		vec!["10.0.0.5".to_string(), "2001:db8::5".to_string()]
	);
}

#[test]
fn per_network_driver_opts_forwarded_as_written_without_priority() {
	use crate::compose::types::ServiceNetworkConfig;
	let mut driver_opts = std::collections::HashMap::new();
	driver_opts.insert("host_interface_name".to_string(), "vethweb".to_string());
	let cfg = ServiceNetworkConfig {
		priority: Some(100),
		driver_opts,
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None, &[]);
	let sent = opts.driver_opts.expect("driver opts present");
	assert_eq!(sent.len(), 1, "priority is not a driver option: {sent:?}");
	assert_eq!(
		sent.get("host_interface_name").map(String::as_str),
		Some("vethweb")
	);
}

#[test]
fn per_network_priority_alone_sends_no_driver_opts() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		priority: Some(7),
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None, &[]);
	assert!(opts.driver_opts.is_none());
	// It travels on the attachment so `network_order` can use it.
	assert_eq!(opts.priority, Some(7));
}

#[test]
fn driver_opts_reach_the_wire_as_options() {
	use crate::compose::types::ServiceNetworkConfig;
	let mut driver_opts = std::collections::HashMap::new();
	driver_opts.insert("host_interface_name".to_string(), "vethweb".to_string());
	let cfg = ServiceNetworkConfig {
		driver_opts,
		..Default::default()
	};
	let opts = build_per_network_options("web", Some(&cfg), None, &[]);
	let wire = serde_json::to_value(&opts).unwrap();
	// libpod reads `options`; under `driver_opts` it decodes and drops it.
	assert_eq!(wire["options"]["host_interface_name"], "vethweb");
	assert!(wire.get("driver_opts").is_none(), "{wire}");
}

#[test]
fn link_aliases_follow_declared_aliases_without_duplicates() {
	use crate::compose::types::ServiceNetworkConfig;
	let cfg = ServiceNetworkConfig {
		aliases: Some(vec!["api".to_string()]),
		..Default::default()
	};
	let links = vec!["database".to_string(), "api".to_string(), "web".to_string()];
	let opts = build_per_network_options("web", Some(&cfg), None, &links);
	assert_eq!(
		opts.aliases,
		vec!["web".to_string(), "api".to_string(), "database".to_string()]
	);
}
