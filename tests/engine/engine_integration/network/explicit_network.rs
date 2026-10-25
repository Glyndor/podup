//! Engine integration tests for a network the compose file declares explicitly.

use super::*;
// ---------------------------------------------------------------------------
// Networks
// ---------------------------------------------------------------------------

#[tokio::test]
async fn explicit_network_created() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("net");
	let engine = Engine::new(client, proj.clone());
	let file = parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    networks:\n      - mynet\nnetworks:\n  mynet:\n    driver: bridge\n",
	)
	.unwrap();

	engine.up(&file).await.unwrap();
	// These two are NOT mutation-proved, and the reason is worth keeping. Both
	// mutations I tried (never creating declared networks, and creating them
	// under a different name) take `up` itself down, because the container
	// references a network that is then missing. So the old `up().unwrap()` did
	// already cover "the network exists under the name the container expects".
	//
	// What these add is the contract in the open: the name carries the project
	// prefix, and the container is actually attached rather than silently left on
	// the default network. They would catch a future change where `up` stops
	// failing on a missing network. Until then they are documentation with teeth,
	// not verified coverage.
	let networks = String::from_utf8_lossy(
		&std::process::Command::new("podman")
			.args(["network", "ls", "--format", "{{.Name}}"])
			.output()
			.expect("podman network ls")
			.stdout,
	)
	.to_string();
	let attached = String::from_utf8_lossy(
		&std::process::Command::new("podman")
			.args([
				"inspect",
				&format!("{proj}-web-1"),
				"--format",
				"{{range $k, $v := .NetworkSettings.Networks}}{{$k}} {{end}}",
			])
			.output()
			.expect("podman inspect")
			.stdout,
	)
	.to_string();
	engine.down(&file).await.unwrap();

	let expected = format!("{proj}_mynet");
	assert!(
		networks.lines().any(|n| n == expected),
		"the declared network {expected} was not created: {networks:?}"
	);
	assert!(
		attached.contains(&expected),
		"the container was not attached to {expected}: {attached:?}"
	);
}
