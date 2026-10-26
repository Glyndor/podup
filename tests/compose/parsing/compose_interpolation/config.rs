use std::fs;
use tempfile::tempdir;

fn config_output(dir: &std::path::Path, files: &[&str]) -> Vec<u8> {
	let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_podup"));
	cmd.current_dir(dir)
		.env("NO_COLOR", "1")
		.env_remove("PODUP_CONFIG_USER_2117")
		.env_remove("PODUP_CONFIG_GREETING_2117")
		.env_remove("COMPOSE_FILE")
		.env_remove("COMPOSE_PROFILES");
	for file in files {
		cmd.args(["-f", file]);
	}
	let out = cmd
		.args(["-p", "interpolation", "config"])
		.output()
		.unwrap();
	assert!(
		out.status.success(),
		"{}",
		String::from_utf8_lossy(&out.stderr)
	);
	out.stdout
}

#[test]
fn config_override_resolves_tagged_scalar() {
	let dir = tempdir().unwrap();
	fs::write(
		dir.path().join("base.yml"),
		"services:\n  web:\n    image: alpine\n    user: '1000'\n",
	)
	.unwrap();
	fs::write(
		dir.path().join("override.yml"),
		"services:\n  web:\n    user: !override '${PODUP_CONFIG_USER_2117:-runner}'\n",
	)
	.unwrap();
	let output = config_output(dir.path(), &["base.yml", "override.yml"]);
	let value: serde_yaml::Value = serde_yaml::from_slice(&output).unwrap();
	assert_eq!(value["services"]["web"]["user"].as_str(), Some("runner"));
}

#[test]
fn ordinary_config_output_matches_original_bytes() {
	let dir = tempdir().unwrap();
	fs::write(dir.path().join("compose.yml"), "services:\n  web:\n    image: alpine:3.20\n    user: '${PODUP_CONFIG_USER_2117:-runner}'\n    command: ['echo', '$$HOME']\n    environment:\n      GREETING: '${PODUP_CONFIG_GREETING_2117:-hello}'\n").unwrap();
	// A Windows checkout may turn the fixture's line endings into CRLF.
	let expected =
		include_str!("../../../fixtures/interpolation/ordinary-config.yml").replace("\r\n", "\n");
	assert_eq!(
		config_output(dir.path(), &["compose.yml"]),
		expected.as_bytes()
	);
}
