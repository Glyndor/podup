//! #1955: `keep-id` replicas all start, with the same full mapping.
//!
//! When Podman's API service receives several `keep-id` creates at once, one
//! container now and then comes back mapped as `1000:0:1` alone. Container
//! GID 0 is then unmapped, crun cannot write the default
//! `net.ipv4.ping_group_range=0 0` sysctl, and the replica stays in `created`.
//! It is a race, so the test runs several `up` cycles; with the creates
//! serialised none of them may show it.
//!
//! Each cycle reads what it needs, tears the project down, and the
//! assertions run at the end, so a failure leaves nothing behind.

use std::process::Command;

use super::{podman, podman_socket_url, proj, run};

const REPLICAS: usize = 4;
const CYCLES: usize = 6;

fn inspect(socket: &str, name: &str, format: &str) -> String {
	Command::new("podman")
		.args(["--url", socket, "inspect", "--format", format, name])
		.output()
		.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
		.unwrap_or_default()
}

/// Whether any `container:host:size` range under `key` covers container ID 0.
fn maps_container_id_zero(mappings_json: &str, key: &str) -> bool {
	let v: serde_json::Value = serde_json::from_str(mappings_json).unwrap_or_default();
	v[key].as_array().is_some_and(|ranges| {
		ranges.iter().filter_map(|r| r.as_str()).any(|r| {
			let mut parts = r.split(':').map(|p| p.parse::<u64>().ok());
			matches!(
				(parts.next(), parts.next(), parts.next()),
				(Some(Some(0)), Some(Some(_)), Some(Some(size))) if size > 0
			)
		})
	})
}

#[tokio::test]
async fn keep_id_replicas_all_start_with_the_same_full_mapping() {
	let Some(socket) = podman_socket_url() else {
		return;
	};
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let project = proj("keepidrepl");
	let compose = dir.path().join("docker-compose.yml");
	std::fs::write(
		&compose,
		format!(
			"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    \
			 user: \"1000:1000\"\n    userns_mode: keep-id\n    stop_grace_period: 0s\n    \
			 deploy:\n      replicas: {REPLICAS}\n"
		),
	)
	.unwrap();
	let c = compose.to_str().unwrap();

	let mut failures = Vec::new();
	for cycle in 1..=CYCLES {
		let up = run(&["-f", c, "-p", &project, "up", "-d"]);
		if !up.status.success() {
			failures.push(format!(
				"cycle {cycle}: up failed: {}",
				String::from_utf8_lossy(&up.stderr)
			));
		}
		let seen: Vec<(String, String, String)> = (1..=REPLICAS)
			.map(|i| {
				let name = format!("{project}-web-{i}");
				let state = inspect(&socket, &name, "{{.State.Status}}");
				let maps = inspect(&socket, &name, "{{json .HostConfig.IDMappings}}");
				(name, state, maps)
			})
			.collect();
		let down = run(&["-f", c, "-p", &project, "down"]);
		if !down.status.success() {
			failures.push(format!(
				"cycle {cycle}: down failed: {}",
				String::from_utf8_lossy(&down.stderr)
			));
		}

		for (name, state, maps) in &seen {
			if state != "running" {
				failures.push(format!("cycle {cycle}: {name} is {state:?}"));
			}
			if maps != &seen[0].2 {
				failures.push(format!(
					"cycle {cycle}: {name} mapping {maps} differs from {} {}",
					seen[0].0, seen[0].2
				));
			}
			if !maps_container_id_zero(maps, "GidMap") {
				failures.push(format!(
					"cycle {cycle}: {name} leaves GID 0 unmapped: {maps}"
				));
			}
		}
	}

	assert!(failures.is_empty(), "{}", failures.join("\n"));
}
