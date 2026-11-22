//! Promotion preserves native value parsing and quoted argument values.

use super::fields_test_support::{render, two_services, two_services_on_front, unit};

fn assert_control_dns_fallback(field: &str, key: &str, flag: &str, value: &str) {
	for blank in [r"\t\0\x01\x7f", r" \t\0\x01 "] {
		let yaml = two_services(&format!("    {field}: [{value:?}, \"{blank}\"]\n"));
		for yaml in [yaml.split("  db:").next().unwrap(), &yaml] {
			let out = render(yaml);
			assert!(!unit(&out, "demo.pod").contents.contains(flag));
			for container in out
				.units
				.iter()
				.filter(|unit| unit.filename.ends_with(".container"))
			{
				let entries: Vec<_> = container
					.contents
					.lines()
					.filter_map(|line| line.strip_prefix(&format!("{key}=")))
					.collect();
				assert_eq!(entries.len(), 2);
				assert_eq!(entries[0], value);
				assert!(entries[1].trim().is_empty());
			}
			assert!(out.warnings.iter().any(|warning| warning == &format!(
				"x-podman-pod: {field}: DNS entries must not be empty or blank; the value stays on each container"
			)));
		}
	}
}

#[test]
fn dns_control_only_entry_stays_on_containers() {
	assert_control_dns_fallback("dns", "DNS", "DNS=", "1.1.1.1");
}

#[test]
fn dns_option_control_only_entry_stays_on_containers() {
	assert_control_dns_fallback("dns_opt", "DNSOption", "DNSOption=", "ndots:1");
}

#[test]
fn dns_search_control_only_entry_stays_on_containers() {
	assert_control_dns_fallback("dns_search", "DNSSearch", "DNSSearch=", "example.com");
}

#[test]
fn container_hostname_trims_native_boundary_spaces() {
	let out = render(&two_services("    hostname: ' shared-host '\n"));
	assert!(!unit(&out, "demo.pod").contents.contains("--hostname="));
	for name in ["demo-web.container", "demo-db.container"] {
		assert!(unit(&out, name).contents.contains("HostName=shared-host\n"));
	}
}

#[test]
fn promoted_dns_trims_native_boundary_spaces() {
	for (field, key, value) in [
		("dns", "DNS", "1.1.1.1"),
		("dns_opt", "DNSOption", "ndots:1"),
		("dns_search", "DNSSearch", "example.com"),
	] {
		let out = render(&two_services(&format!("    {field}: [' {value} ']\n")));
		assert!(unit(&out, "demo.pod")
			.contents
			.contains(&format!("{key}={value}\n")));
		for name in ["demo-web.container", "demo-db.container"] {
			assert!(!unit(&out, name).contents.contains(&format!("{key}=")));
		}
		assert!(out.warnings.is_empty(), "{:?}", out.warnings);
	}
}

fn assert_trimmed_boundary_spaces(yaml: &str, key: &str, value: &str) {
	let expected = format!("{key}={value}\n");
	let out = render(yaml);
	assert!(unit(&out, "demo.pod").contents.contains(&expected));
	for name in ["demo-web.container", "demo-db.container"] {
		assert!(!unit(&out, name).contents.contains(&format!("{key}=")));
	}
}

/// Outside a pod the same value on its own container is written as it is;
/// Quadlet trims it when it reads the unit.
fn assert_native_boundary_spaces(yaml: &str, key: &str, value: &str) {
	let expected = format!("{key}= {value} \n");
	let native = render(&yaml.replace("x-podman-pod: true\n", ""));
	for name in ["demo-web.container", "demo-db.container"] {
		assert!(unit(&native, name).contents.contains(&expected));
	}
}

#[test]
fn promoted_add_host_trims_boundary_spaces() {
	let yaml = two_services("    extra_hosts: [' cache:10.0.0.2 ']\n");
	assert_trimmed_boundary_spaces(&yaml, "AddHost", "cache:10.0.0.2");
	assert_native_boundary_spaces(&yaml, "AddHost", "cache:10.0.0.2");
}

#[test]
fn promoted_alias_trims_boundary_spaces() {
	let yaml =
		two_services_on_front("    networks:\n      front:\n        aliases: [' shared ']\n");
	assert_trimmed_boundary_spaces(&yaml, "NetworkAlias", "shared");
	// Outside the pod the alias goes on the `Network=` option list, which a space
	// would break, so it is left out there and the service name stays.
	let native = render(&yaml.replace("x-podman-pod: true\n", ""));
	let web = unit(&native, "demo-web.container").contents.clone();
	assert!(
		web.contains("Network=demo-front.network:alias=web\n"),
		"{web}"
	);
}
