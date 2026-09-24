//! Live engine test for implicit service dependencies.
//!
//! `volumes_from` and `network_mode: service:X` need the referenced container
//! to exist before the dependent is created. Without implicit-dependency
//! ordering both services land in the same dependency level, start
//! concurrently, and podman returns HTTP 500
//! (`looking up container ... for volumes-from` / `... to share net namespace`).
//! The fixture names are chosen so the alphabetical fallback would start
//! `app` before `zdata` and `web` before `zdb`, which is the order podup
//! would pick if it read only `depends_on`.
//!
//! Reproduced on podup 5.9.10 against Podman 5.7.0; the fix turns each
//! reference into an implicit `depends_on` so the dependent waits for the
//! referenced service.
use super::*;

/// `up -d` on a project whose `network_mode: service:X` reference needs
/// the target to exist before the dependent is created. With implicit
/// dependencies disabled this surfaces as
/// `podman API error (HTTP 500): looking up container to share net
/// namespace with: no container with name or ID "<proj>-zdb-1" found`.
#[tokio::test]
async fn implicit_dependencies_order_referenced_services_first() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("imp");
	let engine = Engine::new(client, proj.clone());
	// Names are alphabetical-reversed on purpose: the dependent comes first
	// when sorted, so a resolver that only reads `depends_on` (and ignores
	// `volumes_from` / `network_mode`) cannot pass this test by luck.
	let file = parse_str(
		"services:\n  \
			zdb:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  \
			zdata:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  \
			web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    network_mode: \"service:zdb\"\n  \
			app:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    volumes_from: [\"zdata\"]\n",
	)
	.unwrap();

	let up_result = engine
		.up_with_options(&file, true, &[], &[], false, false, false, false)
		.await;

	// Snapshot before tearing down: once `down -v` runs the containers are
	// gone and `test_project_container_names` returns empty.
	let mut running = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	running.sort();

	// Tear down on every exit path so a follow-up `up` starts clean.
	let teardown = engine.down_with_options(&file, true).await;

	up_result.unwrap_or_else(|e| panic!("up failed: {e:?}"));
	teardown.unwrap_or_else(|e| panic!("down failed: {e:?}"));

	assert_eq!(
		running,
		vec![
			format!("{proj}-app-1"),
			format!("{proj}-web-1"),
			format!("{proj}-zdata-1"),
			format!("{proj}-zdb-1"),
		],
		"every container should be running after up; got: {running:?}"
	);
}
