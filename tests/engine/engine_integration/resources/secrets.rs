//! Reproduction of the file-edit / recreated-container contract for `file:`
//! secrets. The unit tests pin the hash; this pins the end-to-end behaviour
//! on a real Podman: a service started against `s.txt = v1` must see `v2`
//! after the file is rewritten and `up -d` runs again. Before the fix in
//! `internal/engine/container/resolve.rs` the second `up` left the original
//! container running and the new bytes invisible to the container. The
//! fix folds the file content into the config hash so the second `up`
//! recreates the container. The third assertion pins the inverse: when the
//! file is unchanged, the second and third `up`s must agree, because the
//! label describes the bytes that were uploaded and not a re-read of the
//! host file (which could otherwise flap on a `touch`/rewrite).

use super::*;

/// Podman container id for `name`, or empty when it does not exist. Shared
/// with `scale.rs`'s scale-down preservation test: id equality is the
/// proof that the same container survived, not a recreated one.
///
/// The subprocess must drop `XDG_DATA_HOME` and `TMPDIR`: an inherited
/// `XDG_DATA_HOME` from a test-runner workspace would redirect `podman`
/// to a different graph root than the one the engine actually wrote to,
/// and `TMPDIR` does the same thing in some setups. Both env vars are
/// dropped, not set to a specific value, so the subprocess lands on the
/// user's standard rootless storage at `$HOME/.local/share/containers/storage`.
fn container_id(name: &str) -> String {
	let mut cmd = std::process::Command::new("podman");
	cmd.args(["inspect", "-f", "{{.Id}}", name]);
	cmd.env_remove("XDG_DATA_HOME");
	cmd.env_remove("TMPDIR");
	let out = cmd.output().unwrap();
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Run the steps the issue measured. The marker technique separates the two
/// `up` paths the same way `up_no_recreate_skips_running` does, but here
/// the marker is the secret itself: the secret is read-only by the container,
/// so its bytes only change when the container is recreated and the new
/// Podman-native secret replaces the old one. Skipping the recreate keeps
/// the old bytes; recreating drops the old mount and remounts the new one.
///
/// `v1_first` and `v2_after` are both strict: a missing read is a recreate
/// failure, not a silent skip. The assertion after `engine.down` compares
/// the trimmed bytes against the literal we wrote.
#[tokio::test]
async fn changing_a_file_secret_recreates_with_the_new_bytes() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let secret_path = dir.path().join("s.txt");
	std::fs::write(&secret_path, b"v1").unwrap();

	let proj = proj("fsec-rot");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - s\nsecrets:\n  s:\n    file: {}\n",
		secret_path.display()
	);
	let file = parse_str(&yaml).unwrap();

	engine.up(&file).await.unwrap();
	let cname = format!("{proj}-web-1");
	let v1_first = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/run/secrets/s".into()])
		.await
		.unwrap_or_default();

	std::fs::write(&secret_path, b"v2").unwrap();
	engine.up(&file).await.unwrap();
	let v2_after = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/run/secrets/s".into()])
		.await
		.unwrap_or_default();
	engine.down(&file).await.unwrap();

	assert_eq!(
		v1_first.trim(),
		"v1",
		"the first up did not carry v1 to the container, so the test cannot prove the second up did anything"
	);
	assert_eq!(
		v2_after.trim(),
		"v2",
		"up -d after the file was rewritten kept the old container instead of recreating with the new bytes"
	);
}

/// Pins the inverse of the recreate case: once the bytes have been
/// re-uploaded and the container recreated against the new secret, a
/// subsequent `up -d` without changing the file must NOT recreate again,
/// because the label must describe what was uploaded, not a re-read of
/// the file on disk at label time. Without the recorded-digest thread,
/// `config_hash` re-read the file at label time and could drift from
/// what was actually mounted (e.g. on a host `touch` between the upload
/// and the label build); here the file is left alone, so the only
/// divergence the third `up` could introduce is exactly that drift, and
/// a same-id assertion rules it out.
#[tokio::test]
async fn third_up_without_file_change_keeps_the_same_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let dir = tempfile::tempdir().unwrap();
	let secret_path = dir.path().join("s.txt");
	std::fs::write(&secret_path, b"v1").unwrap();

	let proj = proj("fsec-stable");
	let engine = Engine::with_base_dir(client, proj.clone(), dir.path().to_path_buf());
	let yaml = format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    secrets:\n      - s\nsecrets:\n  s:\n    file: {}\n",
		secret_path.display()
	);
	let file = parse_str(&yaml).unwrap();
	let cname = format!("{proj}-web-1");

	// First up with v1: container is created.
	engine.up(&file).await.unwrap();
	let id_first = container_id(&cname);
	assert!(
		!id_first.is_empty(),
		"first up must create the container {cname}"
	);

	// Rewrite to v2 and up again: container is recreated with the new bytes.
	std::fs::write(&secret_path, b"v2").unwrap();
	engine.up(&file).await.unwrap();
	let id_after_v2 = container_id(&cname);
	assert_ne!(
		id_first, id_after_v2,
		"the file edit must have recreated the container; got id {id_after_v2}"
	);
	let v2_seen = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/run/secrets/s".into()])
		.await
		.unwrap_or_default();
	assert_eq!(
		v2_seen.trim(),
		"v2",
		"the recreated container must see v2, not the stale v1"
	);

	// Third up without touching the file: the label must match what was
	// uploaded last time, so no recreate and the container id survives.
	engine.up(&file).await.unwrap();
	let id_after_third = container_id(&cname);
	assert_eq!(
		id_after_v2, id_after_third,
		"a third up -d without a file change must NOT recreate; got id {id_after_third}, was {id_after_v2}"
	);
	let v2_still = engine
		.test_exec_capture(&cname, vec!["cat".into(), "/run/secrets/s".into()])
		.await
		.unwrap_or_default();
	assert_eq!(
		v2_still.trim(),
		"v2",
		"the surviving container must still see v2, not a phantom rewrite"
	);

	engine.down(&file).await.unwrap();
}
