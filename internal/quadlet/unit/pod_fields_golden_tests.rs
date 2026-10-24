//! Whole-unit output retained from the parent of the pod-field change.

use super::pod_fields_test_support::{pod_contents, render, unit};

#[test]
fn old_pod_inputs_match_parent_output() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    ports: [\"8080:80\"]\n  db:\n    image: postgres\n    ports: [\"5432:5432\"]\nnetworks:\n  front: {}\n";
	let expected = "# podup-owner: demo\n[Pod]\nPodName=demo\nNetwork=demo-front.network\nPublishPort=5432:5432\nPublishPort=8080:80\nPodmanArgs=--add-host=\"db:127.0.0.1\"\nPodmanArgs=--add-host=\"web:127.0.0.1\"\nPodmanArgs=--label=\"podup.project=demo\"\nPodmanArgs=--exit-policy=continue\n";
	assert_eq!(pod_contents(yaml), expected);
}

#[test]
fn non_pod_mixed_mounts_match_parent_output() {
	let yaml = "services:\n  web:\n    image: nginx\n    volumes:\n      - type: tmpfs\n        target: /cache\n      - type: bind\n        source: /srv/data\n        target: /data\n      - type: tmpfs\n        target: /scratch\n      - data:/seed\n      - /private\n    tmpfs: [/run]\nvolumes:\n  data: {}\n";
	let out = render(yaml);
	let expected = "# podup-owner: demo\n[Unit]\nDescription=web (podup)\n\n[Container]\nContainerName=demo-web\nImage=nginx\nTmpfs=/cache\nVolume=/srv/data:/data\nTmpfs=/scratch\nVolume=demo-data.volume:/seed\nVolume=/private\nLabel=podup.project=demo\nLabel=podup.service=web\nTmpfs=/run\nLogDriver=k8s-file\nPodmanArgs=--log-opt=\"max-size=10485760\"\n\n[Install]\nWantedBy=default.target\n";
	assert_eq!(unit(&out, "demo-web.container").contents, expected);
}
