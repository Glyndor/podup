//! Engine integration tests for `port` resolving a published binding.
use super::*;
// ---------------------------------------------------------------------------
// Port
// ---------------------------------------------------------------------------

/// `Engine::port` returns `Result<()>` and writes the binding to stdout, so this
/// can only assert that a published port resolves without error. The binding
/// itself is checked where it is observable, in
/// `stats_flags::cli_port_prints_the_published_binding`; this test used to be
/// named for a return value the API does not have.
#[tokio::test]
async fn engine_port_resolves_a_published_port() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("prt");
	let engine = Engine::new(client, proj.clone());
	// A port chosen at run time, not a constant: three tests shared 18081 and a
	// fourth 18080, so any two running at once lost the bind and failed with
	// `pasta failed ... Address already in use`.
	let port = super::free_port();
	let file = parse_str(&format!(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    ports:\n      - \"127.0.0.1:{port}:80\"\n"
	))
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.port(&file, "web", "80", "tcp").await.unwrap();
	engine.down(&file).await.unwrap();
}
