//! `x-podman-pod: true` against a real Podman: `up` creates one pod holding every
//! service.
use std::process::Command;

use super::pod_fixtures::{down, pod_json, write_compose};
use super::*;
#[tokio::test]
async fn up_with_pod_creates_one_pod_with_every_service() {
	if podman().await.is_none() {
		return;
	}
	let dir = tempfile::tempdir().unwrap();
	let proj = proj("pod-one");
	let compose = write_compose(dir.path(), "");
	let out = Command::new(bin())
		.args(["-f", &compose, "-p", &proj, "up", "-d"])
		.output()
		.unwrap();
	// The progress stream goes to stderr; the two are read together so the
	// assertion does not depend on which one carries a given line.
	let stdout = format!(
		"{}{}",
		String::from_utf8_lossy(&out.stdout),
		String::from_utf8_lossy(&out.stderr)
	);
	let pod = pod_json(&proj);
	down(&compose, &proj);
	assert!(out.status.success(), "up failed:\n{stdout}");
	let pod = pod.expect("the project pod must exist after up");
	assert_eq!(pod["Name"], proj);
	let shared: Vec<String> = pod["SharedNamespaces"]
		.as_array()
		.map(|a| {
			a.iter()
				.filter_map(|v| v.as_str().map(str::to_string))
				.collect()
		})
		.unwrap_or_default();
	assert_eq!(
		shared,
		vec!["net".to_string()],
		"only the network namespace is shared: {pod}"
	);
	let members: Vec<String> = pod["Containers"]
		.as_array()
		.map(|a| {
			a.iter()
				.filter_map(|c| c["Name"].as_str().map(str::to_string))
				.collect()
		})
		.unwrap_or_default();
	assert!(
		members.iter().any(|m| m == &format!("{proj}-web-1"))
			&& members.iter().any(|m| m == &format!("{proj}-db-1")),
		"both services must be members of the pod: {members:?}"
	);
	assert!(
		stdout.contains("Pod") && stdout.contains("Created"),
		"up must report the pod it created:\n{stdout}"
	);
}
