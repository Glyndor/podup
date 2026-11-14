//! Conflict and source-absence tests for pod-shared fields.

use super::fields_test_support::{pod_contents, render, unit};

#[test]
fn different_hostnames_stay_on_containers_without_warning() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    hostname: web-host\n  db:\n    image: postgres\n    hostname: db-host\n";
	let out = render(yaml);
	let pod = unit(&out, "demo.pod").contents.clone();
	let web = unit(&out, "demo-web.container").contents.clone();
	let db = unit(&out, "demo-db.container").contents.clone();

	assert!(
		!pod.contains("--hostname="),
		"pod chose one conflicting hostname in:\n{pod}"
	);
	assert!(
		web.contains("HostName=web-host") && db.contains("HostName=db-host"),
		"containers lost their conflicting hostnames:\n{web}\n{db}"
	);
	assert!(
		out.warnings
			.iter()
			.all(|warning| !warning.contains("hostname")),
		"unexpected hostname disagreement warning: {:?}",
		out.warnings
	);
}

#[test]
fn pod_keeps_exit_policy_and_omits_stop_timeout() {
	let yaml =
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    stop_grace_period: 30s\n";
	let pod = pod_contents(yaml);
	assert!(
		pod.contains("PodmanArgs=--exit-policy=continue") && !pod.contains("ExitPolicy="),
		"pod lost its exit policy:\n{pod}"
	);
	assert!(
		!pod.contains("--stop-timeout=") && !pod.contains("StopTimeout="),
		"pod invented a stop timeout in:\n{pod}"
	);
}

#[test]
fn quoted_scalars_are_not_promoted_and_pod_containers_omit_shm_size() {
	for (field, flag, native) in [
		("userns_mode", "--userns=", "UserNS="),
		("shm_size", "--shm-size=", "ShmSize="),
		("hostname", "--hostname=", "HostName="),
	] {
		let yaml = format!(
			"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    {field}: 'host\"'\n"
		);
		let out = render(&yaml);
		let pod = unit(&out, "demo.pod").contents.clone();
		let web = unit(&out, "demo-web.container").contents.clone();
		assert!(
			!pod.contains(flag),
			"{field} was promoted with a quote in:\n{pod}"
		);
		assert!(
			if field == "shm_size" {
				!web.contains(native)
			} else {
				web.contains(native)
			},
			"unexpected container value for {field}:\n{web}"
		);
	}
}
