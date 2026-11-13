use std::path::Path;

use super::super::super::tests::{report_in, test_home};

// ---------------------------------------------------------------------------
// sensitive_host_source
// ---------------------------------------------------------------------------

/// The reasons of the `sensitive_host_source` findings for `yaml`, with the
/// project in `/srv/app`.
fn sources(yaml: &str) -> Vec<String> {
	report_in(yaml, Path::new("/srv/app"))
		.into_iter()
		.filter(|f| f.check == "sensitive_host_source")
		.map(|f| f.reason)
		.collect()
}

#[test]
fn audit_sensitive_host_source_flags_a_secret_file_the_service_uses() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    secrets: [pw]\n\
	            secrets:\n  pw:\n    file: /etc/shadow\n";
	let reasons = sources(yaml);
	assert_eq!(reasons.len(), 1, "{reasons:#?}");
	assert!(
		reasons[0].contains("a secret's file: /etc/shadow"),
		"{reasons:#?}"
	);
}

#[test]
fn audit_sensitive_host_source_passes_on_a_secret_no_service_uses() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    secrets: [app]\n\
	            secrets:\n  pw:\n    file: /etc/shadow\n  app:\n    file: ./app.key\n";
	assert!(sources(yaml).is_empty(), "{:#?}", sources(yaml));
}

#[test]
fn audit_sensitive_host_source_flags_a_config_and_a_build_secret() {
	let home = test_home();
	let yaml = format!(
		"services:\n  web:\n    image: alpine:3.20\n    configs: [kube]\n\
		 \x20   build:\n      context: .\n      secrets: [aws]\n\
		 configs:\n  kube:\n    file: ~/.kube/config\n\
		 secrets:\n  aws:\n    file: {home}/.aws/credentials\n"
	);
	let reasons = sources(&yaml);
	assert_eq!(reasons.len(), 2, "{reasons:#?}");
	assert!(
		reasons[0].contains("Kubernetes credentials"),
		"{reasons:#?}"
	);
	assert!(
		reasons[1].contains("a build secret's file:"),
		"{reasons:#?}"
	);
}

#[test]
fn audit_sensitive_host_source_passes_on_a_home_file_beside_the_credentials() {
	// `.sshx` is not `.ssh`. Only the credential finding is ruled out: a
	// home under `/root` is sensitive as a whole through the shared list.
	let home = test_home();
	let yaml = format!(
		"services:\n  web:\n    image: alpine:3.20\n    configs: [app]\n\
		 configs:\n  app:\n    file: {home}/.sshx/app.conf\n"
	);
	let reasons = sources(&yaml);
	assert!(
		!reasons
			.iter()
			.any(|r| r.contains("SSH keys") || r.contains("whole home")),
		"{reasons:#?}"
	);
}

#[test]
fn audit_sensitive_host_source_flags_an_env_file_that_climbs_to_etc() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    env_file: ../../etc/sudoers\n";
	let reasons = sources(yaml);
	assert_eq!(reasons.len(), 1, "{reasons:#?}");
	assert!(
		reasons[0].contains("env_file: ../../etc/sudoers resolves to /etc/sudoers"),
		"{reasons:#?}"
	);
}

#[test]
fn audit_sensitive_host_source_passes_on_an_env_file_in_the_project() {
	let yaml =
		"services:\n  web:\n    image: alpine:3.20\n    env_file: [./app.env, ../app/.env]\n";
	assert!(sources(yaml).is_empty(), "{:#?}", sources(yaml));
}

#[test]
fn audit_sensitive_host_source_flags_the_home_as_build_context() {
	let home = test_home();
	for context in ["~".to_string(), home.clone(), format!("{home}/")] {
		let yaml = format!("services:\n  web:\n    build: \"{context}\"\n");
		let reasons = sources(&yaml);
		assert_eq!(reasons.len(), 1, "{context}: {reasons:#?}");
		assert!(
			reasons[0].contains("the whole home directory"),
			"{reasons:#?}"
		);
	}
}

#[test]
fn audit_sensitive_host_source_passes_on_a_project_or_remote_context() {
	for context in [
		".",
		"./web",
		"https://github.com/example/app.git",
		"git@example.com:a/b.git",
	] {
		let yaml = format!("services:\n  web:\n    build: \"{context}\"\n");
		assert!(
			sources(&yaml).is_empty(),
			"{context}: {:#?}",
			sources(&yaml)
		);
	}
}

#[test]
fn audit_sensitive_host_source_flags_a_dockerfile_outside_the_context() {
	// The Dockerfile resolves against the context, not the project.
	let yaml = "services:\n  web:\n    build:\n      context: ./web\n      dockerfile: ../../../etc/passwd\n";
	let reasons = sources(yaml);
	assert_eq!(reasons.len(), 1, "{reasons:#?}");
	assert!(reasons[0].starts_with("build.dockerfile:"), "{reasons:#?}");
	assert!(
		reasons[0].contains("resolves to /etc/passwd"),
		"{reasons:#?}"
	);
}

#[test]
fn audit_sensitive_host_source_passes_on_a_dockerfile_beside_the_context() {
	// Two levels up from `/srv/app/web` is `/srv`, not `/etc`.
	let yaml = "services:\n  web:\n    build:\n      context: ./web\n      dockerfile: ../../Containerfile\n";
	assert!(sources(yaml).is_empty(), "{:#?}", sources(yaml));
}

#[test]
fn audit_sensitive_host_source_flags_a_local_additional_context() {
	let yaml = "services:\n  web:\n    build:\n      context: .\n      additional_contexts:\n        keys: ~/.ssh\n        base: docker-image://alpine:3.20\n";
	let _home = test_home();
	let reasons = sources(yaml);
	assert_eq!(reasons.len(), 1, "{reasons:#?}");
	assert!(
		reasons[0].contains("build.additional_contexts: keys: ~/.ssh resolves to"),
		"{reasons:#?}"
	);
	assert!(reasons[0].contains("SSH keys"), "{reasons:#?}");
}

#[test]
fn audit_sensitive_host_source_passes_on_image_and_service_contexts() {
	let yaml = "services:\n  web:\n    build:\n      context: .\n      additional_contexts:\n        base: docker-image://alpine:3.20\n        lib: service:lib\n        assets: ./assets\n  lib:\n    build: ./lib\n";
	assert!(sources(yaml).is_empty(), "{:#?}", sources(yaml));
}
