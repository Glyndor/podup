//! The compose file and the `podman` reads the `x-podman-pod` tests share.
use super::*;
use std::fs;
use std::process::Command;

pub(super) fn write_compose(dir: &std::path::Path, ports_for_db: &str) -> String {
	let yaml = format!(
		"x-podman-pod: true\nservices:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  db:\n    image: alpine:latest\n    command: [\"sh\", \"-c\", \"while true; do echo hi | nc -l -p 5432; done\"]\n{ports_for_db}"
	);
	let path = dir.path_buf().join("docker-compose.yml");
	fs::write(&path, yaml).unwrap();
	path.to_str().unwrap().to_string()
}

trait PathBufExt {
	fn path_buf(&self) -> std::path::PathBuf;
}
impl PathBufExt for std::path::Path {
	fn path_buf(&self) -> std::path::PathBuf {
		self.to_path_buf()
	}
}

pub(super) fn pod_json(project: &str) -> Option<serde_json::Value> {
	let out = Command::new("podman")
		.args(["pod", "inspect", project])
		.output()
		.ok()?;
	if !out.status.success() {
		return None;
	}
	let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
	// Podman 5 returns an object, Podman 6 an array with one object.
	Some(v.as_array().and_then(|a| a.first().cloned()).unwrap_or(v))
}

pub(super) fn down(compose: &str, proj: &str) {
	let _ = Command::new(bin())
		.args(["-f", compose, "-p", proj, "down", "-v"])
		.output();
}
