//! The exported pod shares only the network, as the pod `up` creates; what
//! that leaves on each container and what stays on the pod.

use super::fields_test_support::{render, unit};

fn units(yaml: &str) -> (String, String, String, String) {
	let out = render(yaml);
	(
		unit(&out, "demo.pod").contents.clone(),
		unit(&out, "demo-web.container").contents.clone(),
		unit(&out, "demo-db.container").contents.clone(),
		out.warnings.join("\n"),
	)
}

#[test]
fn the_pod_shares_only_the_network() {
	let (pod, _, _, _) = units(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n  db:\n    image: postgres\n",
	);
	assert!(pod.lines().any(|l| l == "PodmanArgs=--share=net"), "{pod}");
}

#[test]
fn a_hostname_every_service_agrees_on_stays_on_each_container() {
	// With only the network shared, the pod's hostname never reaches its
	// members (Podman 5.7.0: the member answers with its own id).
	let (pod, web, db, warnings) = units(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    hostname: same\n  db:\n    image: postgres\n    hostname: same\n",
	);
	assert!(!pod.contains("--hostname"), "{pod}");
	assert!(web.lines().any(|l| l == "HostName=same"), "{web}");
	assert!(db.lines().any(|l| l == "HostName=same"), "{db}");
	assert!(!warnings.contains("hostname"), "{warnings}");
}

#[test]
fn different_hostnames_stay_on_their_containers_without_a_warning() {
	let (_, web, db, warnings) = units(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    hostname: w\n  db:\n    image: postgres\n    hostname: d\n",
	);
	assert!(web.lines().any(|l| l == "HostName=w"), "{web}");
	assert!(db.lines().any(|l| l == "HostName=d"), "{db}");
	assert!(!warnings.contains("hostname"), "{warnings}");
}

fn pod_shm(pod: &str) -> Vec<&str> {
	pod.lines().filter(|l| l.contains("--shm-size")).collect()
}

#[test]
fn an_agreed_shm_size_goes_on_the_pod_and_not_on_the_containers() {
	// Podman ignores a member's own --shm-size and mounts the pod's.
	let (pod, web, db, _) = units(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 256m\n  db:\n    image: postgres\n    shm_size: 256m\n",
	);
	let shm = pod_shm(&pod);
	assert_eq!(shm.len(), 1, "{pod}");
	assert!(
		shm[0].contains("256m") || shm[0].contains("268435456"),
		"{pod}"
	);
	assert!(
		!web.contains("ShmSize=") && !db.contains("ShmSize="),
		"{web}\n{db}"
	);
}

#[test]
fn a_service_without_shm_size_does_not_disagree() {
	let (pod, _, db, warnings) = units(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 256m\n  db:\n    image: postgres\n",
	);
	assert_eq!(pod_shm(&pod).len(), 1, "{pod}");
	assert!(!db.contains("ShmSize="), "{db}");
	assert!(!warnings.contains("shm_size"), "{warnings}");
}

#[test]
fn disagreeing_shm_sizes_give_the_pod_the_largest_and_say_so() {
	let (pod, web, db, warnings) = units(
		"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    shm_size: 1g\n  db:\n    image: postgres\n    shm_size: 64m\n",
	);
	let shm = pod_shm(&pod);
	assert_eq!(shm.len(), 1, "{pod}");
	assert!(
		shm[0].contains("1g") || shm[0].contains("1073741824"),
		"{pod}"
	);
	assert!(!shm[0].contains("64m"), "{pod}");
	assert!(
		!web.contains("ShmSize=") && !db.contains("ShmSize="),
		"{web}\n{db}"
	);
	assert!(
		warnings.contains("shm_size") && warnings.contains("web") && warnings.contains("db"),
		"{warnings}"
	);
}
