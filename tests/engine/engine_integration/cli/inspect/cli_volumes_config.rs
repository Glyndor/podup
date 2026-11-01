//! CLI binary integration tests for the two commands that report the project
//! rather than change it: `volumes` and `config`.
use super::*;
use std::fs;
use tempfile::tempdir;
#[tokio::test]
async fn cli_volumes_lists_named_volumes() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-vols", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - data:/data\nvolumes:\n  data:\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	// -q prints only the resolved on-host name ({proj}_data).
	let out = run(&["-f", c, "-p", &proj, "volumes", "-q"]);
	assert!(
		out.status.success(),
		"volumes -q failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let names = String::from_utf8_lossy(&out.stdout);
	assert!(
		names.lines().any(|l| l.trim() == format!("{proj}_data")),
		"volumes -q must list the resolved volume name, got: {names:?}"
	);

	// JSON format carries the same name.
	let json = run(&["-f", c, "-p", &proj, "volumes", "--format", "json"]);
	assert!(
		json.status.success(),
		"volumes --format json failed: {:?}",
		json.stderr
	);
	let parsed: serde_json::Value =
		serde_json::from_str(String::from_utf8_lossy(&json.stdout).trim()).expect("valid JSON");
	assert!(
		parsed
			.as_array()
			.is_some_and(|a| a.iter().any(|v| v["Name"] == format!("{proj}_data"))),
		"volumes --format json must include the volume: {parsed}"
	);
}

#[tokio::test]
async fn cli_config_resolve_image_digests_pins_digest() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let proj = format!("t{}-digests", std::process::id());
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	// Ensure the image (with its registry digest) is present locally.
	run(&["-f", c, "-p", &proj, "pull"]);

	let out = run(&["-f", c, "-p", &proj, "config", "--resolve-image-digests"]);
	assert!(
		out.status.success(),
		"config --resolve-image-digests failed: {:?}",
		out.stderr
	);
	let yaml = String::from_utf8_lossy(&out.stdout);
	assert!(
		yaml.contains("alpine@sha256:"),
		"image must be pinned to a registry digest, got: {yaml}"
	);
}

/// #1184: `config` left `env_file` unresolved, so a service taking its whole
/// environment from a file rendered with no `environment:` at all: the one
/// command you use to ask what will actually run pointed away from the answer.
/// docker compose materialises it and drops the key; measured against
/// docker compose v5.1.3 on this exact input.
#[test]
fn cli_config_materialises_env_file_into_environment() {
	let dir = tempdir().unwrap();
	let proj = format!("t{}-cfgenv", std::process::id());
	fs::write(dir.path().join("app.env"), "FROM_FILE=resolved\n").unwrap();
	let compose = dir.path().join("docker-compose.yml");
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    environment:\n      SHARED: from-service\n    env_file:\n      - app.env\n",
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let out = run(&["-f", c, "-p", &proj, "config"]);
	assert!(
		out.status.success(),
		"config failed: {}",
		String::from_utf8_lossy(&out.stderr)
	);
	let yaml = String::from_utf8_lossy(&out.stdout);

	assert!(
		yaml.contains("FROM_FILE: resolved"),
		"the env_file value was not folded into environment: {yaml}"
	);
	assert!(
		!yaml.contains("env_file"),
		"env_file must be dropped once it has been resolved, like docker compose: {yaml}"
	);
	assert!(
		yaml.contains("SHARED: from-service"),
		"a key set in both places must render the value the container would see: {yaml}"
	);
}
