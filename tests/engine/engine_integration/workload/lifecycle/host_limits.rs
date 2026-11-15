//! Limits the host may not support, against a real daemon: a value the host
//! cannot honour is dropped with a warning instead of keeping the container
//! from starting.
use super::*;

/// The cgroup version the daemon reports (`"v1"` or `"v2"`).
fn cgroups_version() -> String {
	let out = std::process::Command::new("podman")
		.args(["info", "--format", "{{.Host.CgroupsVersion}}"])
		.output()
		.expect("podman info");
	String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[tokio::test]
async fn a_service_with_cgroups_v1_knobs_still_starts() {
	if podman().await.is_none() {
		return;
	}
	// Before the fix `mem_swappiness` reached crun under its real name and
	// stopped the container on cgroups v2 (`cannot set memory swappiness with
	// cgroupv2`), while `oom_kill_disable` and `blkio_config` went out under
	// names libpod ignores.
	let guard = super::DownGuard::new(
		"hostlim",
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    mem_limit: 64m\n    mem_swappiness: 10\n    oom_kill_disable: true\n    blkio_config:\n      weight: 300\n",
	);
	let out = run(&["-f", guard.compose_path(), "-p", guard.name(), "up", "-d"]);
	let stderr = String::from_utf8_lossy(&out.stderr);
	assert!(out.status.success(), "up failed:\n{stderr}");

	let container = format!("{}-web-1", guard.name());
	let state = std::process::Command::new("podman")
		.args([
			"inspect",
			&container,
			"--format",
			"{{.State.Status}} {{.HostConfig.Memory}}",
		])
		.output()
		.expect("podman inspect");
	let state = String::from_utf8_lossy(&state.stdout);
	assert_eq!(state.trim(), format!("running {}", 64 << 20), "{stderr}");

	if cgroups_version() == "v2" {
		for field in ["mem_swappiness", "oom_kill_disable"] {
			assert!(
				stderr.contains(&format!("{field} is discarded")),
				"{field} not reported as discarded:\n{stderr}"
			);
		}
	}
}
