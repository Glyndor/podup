//! Mounts remain private to containers, including common pod-service mounts.

use super::pod_fields_test_support::{render, two_services, unit};

#[test]
fn all_mount_types_stay_on_each_pod_container() {
	let field = "    volumes:\n      - type: tmpfs\n        target: /cache\n      - type: bind\n        source: /srv/data\n        target: /data\n      - data:/seed\n      - /private\n    tmpfs: [/run]\n";
	for services in [
		two_services(field),
		format!("x-podman-pod: true\nservices:\n  web:\n    image: nginx\n{field}"),
	] {
		let out = render(&format!("{services}volumes:\n  data: {{}}\n"));
		let pod = &unit(&out, "demo.pod").contents;
		assert!(!pod.contains("Volume=") && !pod.contains("Tmpfs="), "{pod}");
		for container in out
			.units
			.iter()
			.filter(|unit| unit.filename.ends_with(".container"))
		{
			for mount in [
				"Tmpfs=/cache\n",
				"Volume=/srv/data:/data\n",
				"Volume=demo-data.volume:/seed\n",
				"Volume=/private\n",
				"Tmpfs=/run\n",
			] {
				assert!(
					container.contents.contains(mount),
					"missing {mount} in {}",
					container.contents
				);
			}
		}
	}
}
