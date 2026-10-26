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
	assert_control_dns_fallback("dns", "DNS", "--dns=", "1.1.1.1");
}

#[test]
fn dns_option_control_only_entry_stays_on_containers() {
	assert_control_dns_fallback("dns_opt", "DNSOption", "--dns-option=", "ndots:1");
}

#[test]
fn dns_search_control_only_entry_stays_on_containers() {
	assert_control_dns_fallback("dns_search", "DNSSearch", "--dns-search=", "example.com");
}

#[test]
fn promoted_hostname_trims_native_boundary_spaces() {
	let out = render(&two_services("    hostname: ' shared-host '\n"));
	assert!(unit(&out, "demo.pod")
		.contents
		.contains("PodmanArgs=--hostname=\"shared-host\"\n"));
	for name in ["demo-web.container", "demo-db.container"] {
		assert!(!unit(&out, name).contents.contains("HostName="));
	}
}

#[test]
fn promoted_dns_trims_native_boundary_spaces() {
	for (field, key, flag, value) in [
		("dns", "DNS", "--dns", "1.1.1.1"),
		("dns_opt", "DNSOption", "--dns-option", "ndots:1"),
		("dns_search", "DNSSearch", "--dns-search", "example.com"),
	] {
		let out = render(&two_services(&format!("    {field}: [' {value} ']\n")));
		assert!(unit(&out, "demo.pod")
			.contents
			.contains(&format!("PodmanArgs={flag}=\"{value}\"\n")));
		for name in ["demo-web.container", "demo-db.container"] {
			assert!(!unit(&out, name).contents.contains(&format!("{key}=")));
		}
		assert!(out.warnings.is_empty(), "{:?}", out.warnings);
	}
}

fn assert_quoted_boundary_spaces(yaml: &str, flag: &str, value: &str) {
	let expected = format!("PodmanArgs={flag}=\" {value} \"\n");
	let out = render(yaml);
	assert!(unit(&out, "demo.pod").contents.contains(&expected));
	let native = render(&yaml.replace("x-podman-pod: true\n", ""));
	for name in ["demo-web.container", "demo-db.container"] {
		assert!(unit(&native, name).contents.contains(&expected));
		assert!(!unit(&out, name).contents.contains(flag));
	}
}

#[test]
fn promoted_add_host_keeps_container_quoted_boundary_spaces() {
	assert_quoted_boundary_spaces(
		&two_services("    extra_hosts: [' cache:10.0.0.2 ']\n"),
		"--add-host",
		"cache:10.0.0.2",
	);
}

#[test]
fn promoted_alias_keeps_container_quoted_boundary_spaces() {
	assert_quoted_boundary_spaces(
		&two_services_on_front("    networks:\n      front:\n        aliases: [' shared ']\n"),
		"--network-alias",
		"shared",
	);
}
