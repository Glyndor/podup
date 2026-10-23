//! Shared helpers for pod-field rendering tests.

use crate::compose::parse_str;
use crate::quadlet::{generate_at, QuadletOutput, QuadletUnit};

/// Render one compose document for the `demo` project.
pub(super) fn render(yaml: &str) -> QuadletOutput {
	let file = parse_str(yaml).unwrap();
	crate::validate_config(&file).unwrap();
	generate_at(&file, "demo", std::path::Path::new("/srv/app"))
}

/// Return one rendered unit by file name.
pub(super) fn unit<'a>(out: &'a QuadletOutput, filename: &str) -> &'a QuadletUnit {
	out.units
		.iter()
		.find(|unit| unit.filename == filename)
		.unwrap_or_else(|| panic!("no unit named {filename}"))
}

/// Return the pod unit contents for one compose document.
pub(super) fn pod_contents(yaml: &str) -> String {
	let out = render(yaml);
	unit(&out, "demo.pod").contents.clone()
}

/// Return the first container unit contents for one compose document.
pub(super) fn container_contents(yaml: &str) -> String {
	let out = render(yaml);
	unit(&out, "demo-web.container").contents.clone()
}

/// Build two services that declare the same field block.
pub(super) fn two_services(field: &str) -> String {
	format!(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n{field}  db:\n    image: postgres\n{field}"
	)
}

/// Complete topology for the network-field fixtures, including both IP families.
pub(super) fn two_services_on_front(field: &str) -> String {
	format!("{}networks:\n  front:\n    enable_ipv6: true\n    ipam:\n      config:\n        - subnet: 10.5.0.0/24\n        - subnet: 2001:db8::/64\n", two_services(field))
}
