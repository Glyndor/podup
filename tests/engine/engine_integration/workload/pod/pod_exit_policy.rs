//! `x-podman-pod: true` against a real Podman: the pod outlives its last service.
use std::fs;
use std::process::Command;

use super::pod_fixtures::down;
use super::*;
/// The pod keeps running when its last service container exits: the
/// infra container stays up across the gap, and `podman pod inspect`
/// reports `ExitPolicy=continue`. With `restart: "no"` on the only
/// service and `command: [true]`, the service exits successfully and
/// stays exited, and the pod survives. This pins the promised behaviour;
/// it does not prove the explicit field matters, since the stock
/// `containers.conf` default is `continue` too. The unit tests on the pod
/// create body are what fail without the field.
#[tokio::test]
async fn a_pod_keeps_running_after_its_last_service_exits() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	// Prefix the tag (not the project name) with `qa1971` so the project's
	// resources are easy to find in a `podman pod ls --filter name=qa1971`
	// sweep; the rest of the project name follows the same `t<PID>-` shape
	// the rest of the suite uses, so `proj()` still emits it.
	let proj = proj("qa1971-exit-policy");
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: alpine:latest\n    command: [\"true\"]\n    restart: \"no\"\n";
	let path = dir.path().join("docker-compose.yml");
	fs::write(&path, yaml).unwrap();
	let compose = path.to_str().unwrap().to_string();
	let up = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	assert!(
		up.status.success(),
		"up failed: {}",
		String::from_utf8_lossy(&up.stderr)
	);
	// Wait for the service container to actually exit. Up to 20 seconds:
	// `true` should return almost immediately, but image pulls and the
	// first boot of the container stretch the round trip.
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
	let mut service_exited = false;
	while std::time::Instant::now() < deadline {
		let inspect = Command::new("podman")
			.args([
				"container",
				"inspect",
				&format!("{proj}-web-1"),
				"--format",
				"{{.State.Status}}",
			])
			.output();
		if let Ok(out) = inspect {
			if out.status.success() {
				let state = String::from_utf8_lossy(&out.stdout);
				if state.trim() == "exited" {
					service_exited = true;
					break;
				}
			}
		}
		std::thread::sleep(std::time::Duration::from_millis(200));
	}
	assert!(
		service_exited,
		"the service must reach the `exited` state before the pod can be inspected"
	);
	// Pod's exit policy is `continue`, the value the engine stamps.
	let exit_policy = Command::new("podman")
		.args(["pod", "inspect", &proj, "--format", "{{.ExitPolicy}}"])
		.output()
		.unwrap();
	assert!(
		exit_policy.status.success(),
		"`podman pod inspect --format '{{.ExitPolicy}}' must succeed: {}",
		String::from_utf8_lossy(&exit_policy.stderr)
	);
	assert_eq!(
		String::from_utf8_lossy(&exit_policy.stdout).trim(),
		"continue",
		"the pod must carry exit_policy=continue"
	);
	// Infra container is still running, even though every service has
	// exited. The pod does not tear itself down. The infra container is
	// not named after the project; `podman ps` carries the `IsInfra` flag
	// that separates it from the project's containers.
	let infra_ls = Command::new("podman")
		.args([
			"ps",
			"-a",
			"--filter",
			&format!("pod={proj}"),
			"--format",
			"{{.Names}}\t{{.IsInfra}}",
		])
		.output()
		.unwrap();
	assert!(
		infra_ls.status.success(),
		"`podman ps --filter pod={proj}` must succeed: {}",
		String::from_utf8_lossy(&infra_ls.stderr)
	);
	let infra_name = String::from_utf8_lossy(&infra_ls.stdout)
		.lines()
		.find(|l| l.ends_with("\ttrue"))
		.and_then(|l| l.split('\t').next())
		.map(str::to_string)
		.unwrap_or_default();
	assert!(
		!infra_name.is_empty(),
		"the pod must carry an infra container, got: {}",
		String::from_utf8_lossy(&infra_ls.stdout)
	);
	let infra_state = Command::new("podman")
		.args([
			"container",
			"inspect",
			&infra_name,
			"--format",
			"{{.State.Status}}",
		])
		.output()
		.unwrap();
	assert!(
		infra_state.status.success(),
		"`podman container inspect` on the infra container must succeed: {}",
		String::from_utf8_lossy(&infra_state.stderr)
	);
	assert_eq!(
		String::from_utf8_lossy(&infra_state.stdout).trim(),
		"running",
		"the infra container must stay running while exit_policy is continue"
	);
	down(&compose, &proj);
}
