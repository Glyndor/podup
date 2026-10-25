//! Engine integration tests for the engine calls whose output the library never
//! returns: `ps`, `attach_logs` and `logs`.
use super::*;
// ---------------------------------------------------------------------------
// PS with port bindings
// ---------------------------------------------------------------------------

/// `Engine::ps` prints and returns `Result<()>`, so the port column it exists to
/// render is not reachable from here. `cli_ps_subcommand` asserts the table, and
/// `stats_flags::cli_port_prints_the_published_binding` asserts the binding
/// itself. Kept because it drives the with-ports branch of the same call.
#[tokio::test]
async fn ps_with_port_bindings() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("psb");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    ports:\n      - \"19100:80\"\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.ps(&file).await.unwrap();
	engine.down(&file).await.unwrap();
}

// ---------------------------------------------------------------------------
// Query: attach_logs streaming and logs stderr
// ---------------------------------------------------------------------------

/// `attach_logs` streams to stdout, so what it carries cannot be read from the
/// library. Unlike `attach_logs_empty_attach_returns`, which asserts a bound
/// because "returns immediately" is checkable without the stream, there is
/// nothing here to bound; this one names the content. No CLI test covers
/// attached output either, so the claim in the name is unverified.
#[tokio::test]
async fn attach_logs_streams_container_output() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("als");
	let engine = Engine::new(client, proj.clone());
	// Container writes to stdout and stderr then exits; attach_logs should
	// stream the output and return when the stream ends (join_all completes).
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo out-line; echo err-line >&2\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.attach_logs(&file).await.unwrap();
	let _ = engine.down(&file).await;
}

/// Same: `logs` prints, and whether a container's stderr is interleaved into it
/// is a property of the output. `cli_logs_subcommand` asserts stdout content and
/// the service prefix, but drives a service that writes only to stdout, so the
/// stderr path this test is named for is asserted nowhere.
#[tokio::test]
async fn logs_with_stderr_output() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("lge");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"echo error-msg >&2; sleep infinity\"]\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	engine.logs(&file, Some("web"), false).await.unwrap();
	engine.down(&file).await.unwrap();
}
