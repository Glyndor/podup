//! Engine integration tests for resources declared `external: true`: a volume and
//! a secret Podman must already know about.
use std::fs;

use super::*;
// ---------------------------------------------------------------------------
// External volume skip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_volume_missing_errors_on_up() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exv");
	let engine = Engine::new(client, proj.clone());
	// An external volume that does not exist must surface an error rather than
	// being silently skipped (compose spec requires the resource to exist).
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\nvolumes:\n  extdata-does-not-exist:\n    external: true\n",
	)
	.unwrap();

	// Not printed on failure: CodeQL's `rust/cleartext-logging` flagged the
	// formatted `Result` here, the same shape #1611 split in the other suites.
	let err = engine
		.up(&file)
		.await
		.expect_err("expected ExternalNotFound, got Ok");
	assert!(
		matches!(err, podup::ComposeError::ExternalNotFound(_)),
		"expected ExternalNotFound, got a different ComposeError variant"
	);
}

// ---------------------------------------------------------------------------
// External (Podman-native) secret injection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn external_secret_missing_errors_on_up() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("exsec");
	let engine = Engine::new(client, proj.clone());
	// An `external: true` secret that no `podman secret` backs must fail closed,
	// like an external volume, rather than start a container missing the secret.
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets: [absent-secret]\nsecrets:\n  absent-secret:\n    external: true\n",
	)
	.unwrap();

	// Not printed on failure: CodeQL's `rust/cleartext-logging` flagged the
	// formatted `Result` here, the same shape #1611 split in the other suites.
	let err = engine
		.up(&file)
		.await
		.expect_err("expected ExternalNotFound, got Ok");
	assert!(
		matches!(err, podup::ComposeError::ExternalNotFound(_)),
		"expected ExternalNotFound, got a different ComposeError variant"
	);
}

#[cfg(feature = "test-helpers")]
#[tokio::test]
async fn external_secret_injected_into_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("insec");
	let secret_name = format!("{proj}-tok");

	// Create the backing Podman secret out-of-band, the external-secret idiom.
	// Skip the test if the podman CLI is unavailable (socket alone is not enough).
	let dir = tempfile::tempdir().unwrap();
	let secret_src = dir.path().join("tok");
	fs::write(&secret_src, b"native-secret-value").unwrap();
	let created = std::process::Command::new("podman")
		.args([
			"secret",
			"create",
			&secret_name,
			secret_src.to_str().unwrap(),
		])
		.status();
	match created {
		Ok(s) if s.success() => {}
		_ => return,
	}

	// The compose name is `tok` (→ /run/secrets/tok); the actual secret is named
	// differently, exercising the source/target split.
	let yaml = format!(
		"services:\n  app:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets: [tok]\nsecrets:\n  tok:\n    external: true\n    name: {secret_name}\n"
	);
	let file = parse_str(&yaml).unwrap();
	let engine = Engine::new(client, proj.clone());
	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-app-1");
	let out = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/run/secrets/tok".into()])
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();
	let _ = std::process::Command::new("podman")
		.args(["secret", "rm", &secret_name])
		.status();

	assert!(
		out.contains("native-secret-value"),
		"external secret was not injected at /run/secrets/tok: {out:?}"
	);
}
