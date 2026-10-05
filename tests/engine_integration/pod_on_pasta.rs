//! A pod on `network_mode: pasta` against a live Podman (#1994, item 2).
//!
//! When every service of an `x-podman-pod: true` project declares the same
//! `network_mode: "pasta:-m,1400"`, the pod is created on pasta and its
//! members share the namespace: web's port 80 is reachable on `localhost`
//! from the sibling, the non-loopback interface inside the sibling carries
//! pasta's MTU of 1400, and the host's published port is reachable from
//! the host network. Validated by:
//!
//! 1. `podman exec <client> wget -qO- http://127.0.0.1:80` returns the
//!    nginx page (members share localhost).
//! 2. `for i in /sys/class/net/*; do ... mtu; done` inside `<client>`
//!    reports `1400` for the pasta interface (pasta's `-m 1400` option).
//! 3. `std::net::TcpStream::connect` to `127.0.0.1:<port>` from the host
//!    succeeds (the host reaches the published port).
use std::fs;
use std::process::Command;
use std::time::Duration;

use super::*;

#[tokio::test]
async fn up_creates_a_pod_on_pasta_with_agreed_options() {
	if podman().await.is_none() {
		return;
	}
	let Some(socket) = podman_socket_url() else {
		return;
	};
	let dir = tempfile::tempdir().expect("tempdir");
	let host_port = free_port();
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		format!(
			"x-podman-pod: true\nservices:\n  web:\n    image: docker.io/library/nginx:alpine\n    network_mode: \"pasta:-m,1400\"\n    ports:\n      - \"127.0.0.1:{host_port}:80\"\n  client:\n    image: docker.io/library/alpine:3.20\n    command: [\"sleep\", \"300\"]\n    network_mode: \"pasta:-m,1400\"\n"
		),
	)
	.expect("write compose");
	let project = proj("pod-pasta");
	let run = |args: &[&str]| {
		Command::new(bin())
			.args(["-f", compose.to_str().unwrap(), "-p", &project])
			.args(args)
			.env("PODMAN_SOCKET", &socket)
			.output()
			.expect("run podup")
	};
	struct Down<F: Fn()>(F);
	impl<F: Fn()> Drop for Down<F> {
		fn drop(&mut self) {
			(self.0)();
		}
	}
	let _down = Down(|| {
		let _ = run(&["down", "-t", "0"]);
	});

	let up = run(&["up", "-d"]);
	assert!(
		up.status.success(),
		"up with a pod on pasta:-m,1400 failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);

	// `up -d` returns before nginx is listening; poll the page until it
	// answers or the deadline elapses, the way the rest of the suite
	// handles a published-port race. Without the poll the first exec
	// races against nginx's start and reads ECONNREFUSED, which `wget`
	// surfaces as exit 125.
	let deadline = std::time::Instant::now() + Duration::from_secs(30);
	let page;
	loop {
		let out = Command::new("podman")
			.args([
				"--url",
				&socket,
				"exec",
				&format!("{project}-client-1"),
				"wget",
				"-qO-",
				"http://127.0.0.1:80",
			])
			.output()
			.expect("run podman exec");
		if out.status.success() {
			page = String::from_utf8_lossy(&out.stdout).to_string();
			break;
		}
		if std::time::Instant::now() >= deadline {
			panic!(
				"client never reached web on 127.0.0.1:80: stderr={}",
				String::from_utf8_lossy(&out.stderr)
			);
		}
		std::thread::sleep(Duration::from_millis(200));
	}
	assert!(
		page.to_lowercase().contains("nginx"),
		"client must reach web on 127.0.0.1:80, got body={page:?}"
	);

	// pasta's `-m 1400` option sets the container interface's MTU to
	// 1400, the same way `tests::engine_integration::network_pasta` does
	// for a non-pod service. Skip loopback.
	let mtus = podman_cmd(
		&socket,
		&[
			"exec",
			&format!("{project}-client-1"),
			"sh",
			"-c",
			"for i in /sys/class/net/*; do [ \"${i##*/}\" = lo ] || cat \"$i/mtu\"; done",
		],
	);
	assert_eq!(
		mtus.trim(),
		"1400",
		"pasta's -m 1400 should set the pod's interface MTU; got {mtus:?}"
	);

	// The host reaches the published port. `std::net::TcpStream::connect`
	// fails with a connect error if the bridge is in the way, so a clean
	// connect is what the user sees from outside.
	let host_reached = std::net::TcpStream::connect_timeout(
		&std::net::SocketAddr::from(([127, 0, 0, 1], host_port)),
		Duration::from_secs(5),
	);
	assert!(
		host_reached.is_ok(),
		"host TCP connect to 127.0.0.1:{host_port} failed: {:?}",
		host_reached.err(),
	);
}

/// `run --no-deps` reaches the pod without going through `up`, so it must run
/// the same pre-flight: services that disagree on the network mode are
/// refused before any pod or container is created.
#[tokio::test]
async fn run_refuses_a_pod_whose_services_disagree_on_the_network_mode() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().expect("tempdir");
	let compose = dir.path().join("compose.yaml");
	fs::write(
		&compose,
		"x-podman-pod: true\nservices:\n  web:\n    image: docker.io/library/alpine:3.20\n    network_mode: \"pasta:-4\"\n  db:\n    image: docker.io/library/alpine:3.20\n    network_mode: \"pasta:-6\"\n",
	)
	.expect("write compose");
	let project = proj("pod-pasta-run");
	let out = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &project])
		.args(["run", "--rm", "--no-deps", "db", "true"])
		.output()
		.expect("run podup");
	let stderr = String::from_utf8_lossy(&out.stderr);
	let _ = Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&project,
			"down",
			"-t",
			"0",
		])
		.output();
	assert!(!out.status.success(), "run must be refused: {stderr}");
	assert!(stderr.contains("differs from service"), "{stderr}");
}
