//! Shared helpers for the `.build` unit tests.
//!
//! The tests of the compose `build:` fields all render the same unit and
//! read the same warnings, so the plumbing lives here and each test file
//! keeps only its own cases.

use std::path::Path;

use crate::parse_str;
use crate::quadlet::generate_at;

/// Render the `.build` unit of the `app` service in `yaml`, with the compose
/// file's directory at `/srv/app`.
pub(super) fn build_unit(yaml: &str) -> String {
	build_unit_at(yaml, "/srv/app")
}

/// Render the `.build` unit of the `app` service in `yaml`, with the compose
/// file's directory at `base_dir`.
pub(super) fn build_unit_at(yaml: &str, base_dir: &str) -> String {
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", Path::new(base_dir));
	out.units
		.iter()
		.find(|u| u.filename == "proj-app.build")
		.expect("a service with a build block emits a .build unit")
		.contents
		.clone()
}

/// Render the `.build` unit of a service whose `build:` block is `body`,
/// with every line of the body indented under the `build:` key.
pub(super) fn build_unit_for(body: &str) -> String {
	let indented: String = body.lines().map(|l| format!("      {l}\n")).collect();
	build_unit(&format!(
		"services:\n  app:\n    build:\n      context: .\n{indented}"
	))
}

/// The warnings the export produced for `yaml`.
pub(super) fn warnings(yaml: &str) -> Vec<String> {
	let file = parse_str(yaml).unwrap();
	generate_at(&file, "proj", Path::new("/srv/app")).warnings
}
