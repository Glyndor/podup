//! Engine integration tests for a config declared `external: true` and backed by a
//! Podman-native secret.
use std::fs;

use super::*;
// ---------------------------------------------------------------------------
// External (Podman-native) config injection
// ---------------------------------------------------------------------------

#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn external_config_injected_into_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("incfg");
	let secret_name = format!("{proj}-cfg");

	// External configs are backed by Podman secrets too; create one out-of-band.
	let dir = tempfile::tempdir().unwrap();
	let src = dir.path().join("cfg");
	fs::write(&src, b"native-config-value").unwrap();
	match std::process::Command::new("podman")
		.args(["secret", "create", &secret_name, src.to_str().unwrap()])
		.status()
	{
		Ok(s) if s.success() => {}
		_ => return,
	}

	// A long-form absolute target must land the config at that exact path, not
	// under /run/secrets, the config-specific behaviour.
	let yaml = format!(
		"services:\n  app:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    configs:\n      - source: cfg\n        target: /etc/app.conf\nconfigs:\n  cfg:\n    external: true\n    name: {secret_name}\n"
	);
	let file = parse_str(&yaml).unwrap();
	let engine = Engine::new(client, proj.clone());
	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-app-1");
	let out = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/etc/app.conf".into()])
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();
	let _ = std::process::Command::new("podman")
		.args(["secret", "rm", &secret_name])
		.status();

	assert!(
		out.contains("native-config-value"),
		"external config was not injected at /etc/app.conf: {out:?}"
	);
}
