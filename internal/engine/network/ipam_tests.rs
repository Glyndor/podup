use super::*;

#[test]
fn lease_range_ipv4_reserves_network_and_broadcast() {
	let lr = lease_range_from_cidr("172.28.5.0/24").unwrap();
	assert_eq!(lr.start_ip.as_deref(), Some("172.28.5.1"));
	assert_eq!(lr.end_ip.as_deref(), Some("172.28.5.254"));
}

#[test]
fn lease_range_ipv4_slash31_uses_both_addresses() {
	let lr = lease_range_from_cidr("10.0.0.0/31").unwrap();
	assert_eq!(lr.start_ip.as_deref(), Some("10.0.0.0"));
	assert_eq!(lr.end_ip.as_deref(), Some("10.0.0.1"));
}

#[test]
fn lease_range_ipv6_full_span() {
	let lr = lease_range_from_cidr("2001:db8::/120").unwrap();
	assert_eq!(lr.start_ip.as_deref(), Some("2001:db8::"));
	assert_eq!(lr.end_ip.as_deref(), Some("2001:db8::ff"));
}

#[test]
fn lease_range_invalid_cidr_is_none() {
	assert!(lease_range_from_cidr("not-a-cidr").is_none());
	assert!(lease_range_from_cidr("10.0.0.0/40").is_none());
}

#[test]
fn ipam_options_include_driver_and_options() {
	use crate::compose::types::IpamConfig;
	let ipam = IpamConfig {
		driver: Some("host-local".into()),
		options: [("foo".to_string(), "bar".to_string())].into(),
		..Default::default()
	};
	let opts = build_ipam_options(&ipam);
	assert_eq!(opts.get("driver").map(String::as_str), Some("host-local"));
	assert_eq!(opts.get("foo").map(String::as_str), Some("bar"));
}

#[test]
fn build_subnets_maps_pool_fields_and_ip_range() {
	use crate::compose::types::{IpamConfig, IpamPool};
	let ipam = IpamConfig {
		config: vec![IpamPool {
			subnet: Some("172.28.0.0/16".to_string()),
			gateway: Some("172.28.0.1".to_string()),
			ip_range: Some("172.28.5.0/24".to_string()),
			..Default::default()
		}],
		..Default::default()
	};
	let subnets = build_subnets(&ipam);
	assert_eq!(subnets.len(), 1);
	assert_eq!(subnets[0].subnet.as_deref(), Some("172.28.0.0/16"));
	assert_eq!(subnets[0].gateway.as_deref(), Some("172.28.0.1"));
	// The ip_range CIDR is translated into a Podman lease range.
	let lr = subnets[0]
		.lease_range
		.as_ref()
		.expect("lease range derived");
	assert_eq!(lr.start_ip.as_deref(), Some("172.28.5.1"));
	assert_eq!(lr.end_ip.as_deref(), Some("172.28.5.254"));
}

#[test]
fn build_subnets_without_ip_range_has_no_lease_range() {
	use crate::compose::types::{IpamConfig, IpamPool};
	let ipam = IpamConfig {
		config: vec![IpamPool {
			subnet: Some("10.0.0.0/24".to_string()),
			..Default::default()
		}],
		..Default::default()
	};
	let subnets = build_subnets(&ipam);
	assert!(subnets[0].lease_range.is_none());
}

#[test]
fn lease_range_ipv6_zero_prefix_spans_whole_space() {
	// A `/0` IPv6 range uses an all-zero mask: the start is `::` and the end is
	// the maximum address.
	let lr = lease_range_from_cidr("::/0").unwrap();
	assert_eq!(lr.start_ip.as_deref(), Some("::"));
	assert_eq!(
		lr.end_ip.as_deref(),
		Some("ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")
	);
}

#[test]
fn lease_range_ipv6_rejects_oversized_prefix() {
	assert!(lease_range_from_cidr("2001:db8::/129").is_none());
}
