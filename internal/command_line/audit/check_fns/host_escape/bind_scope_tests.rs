use std::path::Path;

use super::super::super::tests::{report_in, test_home};

// ---------------------------------------------------------------------------
// bind_outside_project
// ---------------------------------------------------------------------------

/// The reasons of the `bind_outside_project` findings for `yaml`, with the
/// project in `base_dir`.
fn outside(yaml: &str, base_dir: &Path) -> Vec<String> {
	report_in(yaml, base_dir)
		.into_iter()
		.filter(|f| f.check == "bind_outside_project")
		.map(|f| f.reason)
		.collect()
}

#[test]
fn audit_bind_outside_project_flags_relative_binds_that_climb_out() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n\
	            \x20     - ../shared:/shared\n\
	            \x20     - ./data/../../other:/other:ro\n\
	            \x20     - type: bind\n        source: ../long\n        target: /long\n";
	let reasons = outside(yaml, Path::new("/srv/app"));
	assert_eq!(reasons.len(), 3, "{reasons:#?}");
	assert!(
		reasons[0]
			.contains("../shared resolves to /srv/shared, outside the project directory /srv/app"),
		"{reasons:#?}"
	);
}

#[test]
fn audit_bind_outside_project_passes_on_binds_inside_and_absolute_ones() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n\
	            \x20     - ./data:/data\n\
	            \x20     - ./a/../b:/b\n\
	            \x20     - /srv/shared:/shared\n\
	            \x20     - named:/named\n\
	            \x20     - type: bind\n        source: data\n        target: /long\n\
	            volumes:\n  named:\n";
	let reasons = outside(yaml, Path::new("/srv/app"));
	assert!(reasons.is_empty(), "{reasons:#?}");
}

#[test]
fn audit_bind_outside_project_flags_a_home_bind_that_lands_elsewhere() {
	let home = test_home();
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - ~/notes:/notes\n";
	// The test project sits under a directory that never holds a home.
	let reasons = outside(yaml, Path::new("/nonexistent/project"));
	assert_eq!(reasons.len(), 1, "{reasons:#?}");
	assert!(
		reasons[0].contains(&format!("resolves to {home}/notes")),
		"{reasons:#?}"
	);
}

#[test]
fn audit_bind_outside_project_passes_on_a_home_bind_inside_the_project() {
	let home = test_home();
	let yaml =
		"services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - ~/app/data:/data\n";
	let reasons = outside(yaml, &Path::new(&home).join("app"));
	assert!(reasons.is_empty(), "{reasons:#?}");
}

#[test]
fn audit_bind_outside_project_flags_a_symbolic_link_out_of_the_project() {
	let project = tempfile::tempdir().unwrap();
	let elsewhere = tempfile::tempdir().unwrap();
	std::os::unix::fs::symlink(elsewhere.path(), project.path().join("link")).unwrap();
	std::fs::create_dir(project.path().join("real")).unwrap();
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n\
	            \x20     - ./link/data:/data\n\
	            \x20     - ./real/data:/real\n";
	let reasons = outside(yaml, project.path());
	assert_eq!(reasons.len(), 1, "{reasons:#?}");
	assert!(
		reasons[0].contains("./link/data goes through a symbolic link"),
		"{reasons:#?}"
	);
}

#[test]
fn audit_bind_outside_project_passes_on_a_symbolic_link_that_stays_inside() {
	let project = tempfile::tempdir().unwrap();
	std::fs::create_dir(project.path().join("real")).unwrap();
	std::os::unix::fs::symlink(project.path().join("real"), project.path().join("link")).unwrap();
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - ./link:/data\n";
	let reasons = outside(yaml, project.path());
	assert!(reasons.is_empty(), "{reasons:#?}");
}
