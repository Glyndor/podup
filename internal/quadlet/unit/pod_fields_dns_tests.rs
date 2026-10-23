//! DNS list resets must retain their container-level meaning.

use super::pod_fields_test_support::{render, two_services, unit};

fn assert_dns_fallback(field: &str, key: &str, flag: &str, value: &str) {
	for (blank, rendered) in [("", ""), (" ", " "), ("\t", "")] {
		let block = format!("    {field}: [{value:?}, {blank:?}]\n");
		let out = render(&two_services(&block));
		let pod = &unit(&out, "demo.pod").contents;
		assert!(!pod.contains(flag), "pod promoted a DNS reset: {pod}");
		for name in ["demo-web.container", "demo-db.container"] {
			let container = &unit(&out, name).contents;
			assert!(
				container.contains(&format!("{key}={value}\n")),
				"{container}"
			);
			assert!(
				container.contains(&format!("{key}={rendered}\n")),
				"{container}"
			);
		}
		assert!(
			out.warnings
				.iter()
				.any(|warning| warning.contains(field)
					&& warning.contains("stays on each container")),
			"{:?}",
			out.warnings
		);
	}
}

#[test]
fn dns_with_blank_entry_stays_on_containers() {
	assert_dns_fallback("dns", "DNS", "--dns=", "1.1.1.1");
}

#[test]
fn dns_options_with_blank_entry_stay_on_containers() {
	assert_dns_fallback("dns_opt", "DNSOption", "--dns-option=", "ndots:1");
}

#[test]
fn dns_search_with_blank_entry_stays_on_containers() {
	assert_dns_fallback("dns_search", "DNSSearch", "--dns-search=", "example.com");
}

fn assert_dns_disagreement(field: &str, key: &str, flag: &str, value: &str, other: &str) {
	for second in [format!("    {field}: [{other:?}]\n"), String::new()] {
		for reverse in [false, true] {
			let first = format!("    {field}: [{value:?}]\n");
			let (web, db) = if reverse {
				(&second, &first)
			} else {
				(&first, &second)
			};
			let yaml = format!("x-podman-pod: true\nservices:\n  web:\n    image: nginx\n{web}  db:\n    image: postgres\n{db}");
			let out = render(&yaml);
			assert!(!unit(&out, "demo.pod").contents.contains(flag));
			let name = if reverse {
				"demo-db.container"
			} else {
				"demo-web.container"
			};
			assert!(unit(&out, name)
				.contents
				.contains(&format!("{key}={value}\n")));
			assert!(
				out.warnings
					.iter()
					.any(|warning| warning.contains(field) && warning.contains("services disagree")),
				"{:?}",
				out.warnings
			);
		}
	}
}

#[test]
fn dns_disagreement_stays_on_containers() {
	assert_dns_disagreement("dns", "DNS", "--dns=", "1.1.1.1", "9.9.9.9");
}

#[test]
fn dns_option_disagreement_stays_on_containers() {
	assert_dns_disagreement(
		"dns_opt",
		"DNSOption",
		"--dns-option=",
		"ndots:1",
		"ndots:2",
	);
}

#[test]
fn dns_search_disagreement_stays_on_containers() {
	assert_dns_disagreement(
		"dns_search",
		"DNSSearch",
		"--dns-search=",
		"example.com",
		"example.net",
	);
}
