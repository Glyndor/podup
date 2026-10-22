//! Engine integration tests for `top` and `images`, including the project whose
//! one service has already exited.
use super::*;
// ---------------------------------------------------------------------------
// Top
// ---------------------------------------------------------------------------

/// `Engine::top` returns `Result<()>` and writes the process table to stdout, so
/// at this level the only thing to assert is that asking a running project does
/// not error. What it prints is checked in `cli_top_images::cli_top_subcommand`,
/// which reads the container name, the header and the process actually running.
#[tokio::test]
async fn engine_top_running_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("top");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.top(&file, &[]).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------

/// Same shape: `Engine::images` prints and returns `Result<()>`. The output is
/// asserted in `cli_top_images::cli_images_subcommand`.
#[tokio::test]
async fn engine_images_lists_service_images() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("img");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.images(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

/// #1250: a project where one service has already run to completion (a
/// `migrate` that exits 0 is the everyday shape) used to abort `top` on the
/// stopped container, losing the services it had not reached yet and exiting
/// non-zero. Measured against `docker compose top` v5.1.3 on the same Podman
/// socket: it omits the stopped service, prints the rest and exits 0.
#[tokio::test]
async fn top_skips_a_stopped_service_and_reports_the_rest() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("tss");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  migrate:\n    image: alpine:latest\n    command: [\"true\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();

	// `migrate` has to have actually exited before `top` runs, or this passes for
	// the wrong reason: `up` returns once the container is started, not once it
	// is done, so without this the test could exercise two running services and
	// prove nothing. `wait` blocks until it stops, which is deterministic where
	// a sleep is not.
	engine
		.wait_services_with_options(&file, &["migrate".to_string()], false)
		.await
		.unwrap();

	engine
		.top(&file, &[])
		.await
		.expect("top must skip the stopped service rather than abort on it");

	engine.down(&file).await.unwrap();
}
