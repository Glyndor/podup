//! Mask failures must precede the first transport operation for each entry.

use crate::compose::types::ComposeFile;
use crate::engine::{Engine, RunOptions, RunOverrides};
use crate::error::{ComposeError, Result};
use crate::libpod::{Client, PodmanError};

fn fixture() -> (Engine, ComposeFile) {
	let mut file =
		crate::parse_str("services:\n  worker:\n    image: alpine\nx-podman-pod: true\n").unwrap();
	file.services["worker"].umask = Some("${MASK}".into());
	crate::compose::normalize_default_network(&mut file);
	let engine = Engine::with_base_dir(
		Client::closed_for_preflight(),
		"probe".into(),
		std::env::temp_dir(),
	);
	(engine, file)
}

fn refused(result: Result<()>) {
	assert!(
		matches!(result, Err(ComposeError::Podman(PodmanError::Field {
		service, field, ..
	})) if service == "worker" && field == "umask"),
		"expected mask rejection before transport"
	);
}

#[tokio::test]
async fn umask_up_rejects_before_transport() {
	let (engine, file) = fixture();
	refused(
		engine
			.up_with_options(&file, true, &[], &[], false, true, false, false)
			.await,
	);
}

#[tokio::test]
async fn umask_create_rejects_before_transport() {
	let (engine, file) = fixture();
	refused(
		engine
			.create_with_options(&file, &[], &[], false, true, false)
			.await,
	);
}

#[tokio::test]
async fn umask_scale_rejects_before_transport() {
	let (engine, file) = fixture();
	refused(engine.scale(&file, &[("worker".into(), 2)]).await);
}

#[tokio::test]
async fn umask_run_no_deps_rejects_before_transport() {
	let (engine, file) = fixture();
	let engine = engine.with_run_overrides(RunOverrides {
		no_deps: true,
		..Default::default()
	});
	refused(engine.run(&file, "worker", RunOptions::default()).await);
}
