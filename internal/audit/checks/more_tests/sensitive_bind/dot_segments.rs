use super::*;

fn fires(spec: &str) -> bool {
	let yaml =
		format!("services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - \"{spec}\"\n");
	report_for(&yaml)
		.iter()
		.any(|f| f.check == "sensitive_bind_mount")
}

#[test]
fn dot_and_dot_dot_components_are_folded_before_the_comparison() {
	for spec in [
		"/tmp/../etc:/m",
		"/./etc:/m",
		"/etc/.:/m",
		"/etc/../etc:/m",
		"/usr/../root:/m",
		"/home/../proc:/m",
		"/a/b/../../sys/./x:/m",
		"/var/run/../run/docker.sock:/s",
		"/run/./user/1000/../1000/podman/podman.sock:/s",
		"/..:/m",
		"/../..:/m",
		"/etc/ssh/..:/m",
		"//tmp//..//etc/:/m",
	] {
		assert!(fires(spec), "{spec} lands on a sensitive path");
	}
}

#[test]
fn a_dot_dot_that_leaves_a_sensitive_path_is_not_flagged() {
	for spec in [
		"/etc/../srv/app:/m",
		"/proc/../home/user:/m",
		"/root/../srv:/m",
		"/srv/./app/../data:/m",
		"/tmp/./x:/m",
	] {
		assert!(!fires(spec), "{spec} lands outside the list");
	}
}

#[test]
fn the_long_form_is_folded_too() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    volumes:\n      - type: bind\n        source: /tmp/../etc\n        target: /m\n";
	assert!(report_for(yaml)
		.iter()
		.any(|f| f.check == "sensitive_bind_mount"));
}
