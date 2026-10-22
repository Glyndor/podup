//! Engine integration tests for re-running `up` over a project that already
//! exists.
use super::*;
// Seven assertions in this suite deliberately do not print the `Result` they
// assert on, while others still do. `rust/cleartext-logging` takes its sources
// from the NAME of the called function, so `create_project_secrets` is a taint
// source and those seven were the sinks it reached through `?`. Nothing
// syntactic separates them from the rest, so this note is what keeps the
// inconsistency from reading as an oversight somebody tidies up. #1599.

// ---------------------------------------------------------------------------
// Idempotent re-up over an existing named volume
// ---------------------------------------------------------------------------

#[tokio::test]
async fn up_is_idempotent_over_existing_named_volume() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let compose_body = "services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes:\n      - data:/data\nvolumes:\n  data:\n";
	// `engine.down` keeps named volumes by design, which is the right shape
	// for the engine but leaves `<proj>_data` on the host for this test in
	// particular — the volume is stood up, asserted against, and never asked
	// for again. The CLI's `down -v` is the explicit teardown that reaps
	// both the network and the volume, and the guard makes it fire on drop.
	let _down = super::DownGuard::new("idv", compose_body);
	let proj = _down.name();
	let engine = Engine::new(client, proj.to_owned());
	let file = parse_str(compose_body).unwrap();

	engine.up(&file).await.unwrap();
	// A second `up` must succeed even though the named volume already exists.
	// Podman's libpod volume-create returns HTTP 500 (not 409) for a duplicate
	// name, so a re-up previously aborted here.
	let second = engine.up(&file).await;
	engine.down(&file).await.unwrap();
	second.expect("second up over an existing named volume must be idempotent");
}

// ---------------------------------------------------------------------------
// up -V/--renew-anon-volumes and up --timestamps
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_up_renew_anon_volumes_recreates_cleanly() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("renew");
	// `with_renew_anon_volumes` makes the recreate delete drop old anon volumes
	// (v=true); a forced re-up must still succeed.
	let engine = Engine::new(client, proj.clone()).with_renew_anon_volumes(true);
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// Force recreate: the v=true delete path runs for the existing container.
	let second = engine
		.up_with_options(&file, false, &[], &[], false, true, false, false)
		.await;
	engine.down(&file).await.unwrap();
	second.expect("forced re-up with --renew-anon-volumes must succeed");
}
