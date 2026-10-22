//! Engine integration tests for `ps`.
use super::*;
/// `Engine::ps` returns `Result<()>` and writes the table to stdout, so at this
/// level the only thing to assert is that listing a running project does not
/// error. What it prints is checked where it is reachable, in
/// `cli_ps::cli_ps_subcommand`, which reads the container name and the
/// STATUS column, the column #590 left empty for every container.
#[tokio::test]
async fn ps_shows_running_container() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("ps");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.ps(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}
