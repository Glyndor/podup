//! Whole-unit output for pod sharing and standalone container mounts.

use super::fields_test_support::{pod_contents, render, unit};

#[test]
fn pod_output_shares_only_the_network() {
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    ports: [\"8080:80\"]\n  db:\n    image: postgres\n    ports: [\"5432:5432\"]\nnetworks:\n  front: {}\n";
	let expected = "# podup-owner: demo\n[Pod]\nPodName=demo\nPodmanArgs=--share=net\nNetwork=demo-front.network\nPublishPort=5432:5432\nPublishPort=8080:80\nAddHost=db:127.0.0.1\nAddHost=web:127.0.0.1\nLabel=podup.project=demo\nExitPolicy=continue\n";
	assert_eq!(pod_contents(yaml), expected);
}

#[test]
fn non_pod_mixed_mounts_match_parent_output() {
	let yaml = "services:\n  web:\n    image: nginx\n    volumes:\n      - type: tmpfs\n        target: /cache\n      - type: bind\n        source: /srv/data\n        target: /data\n      - type: tmpfs\n        target: /scratch\n      - data:/seed\n      - /private\n    tmpfs: [/run]\nvolumes:\n  data: {}\n";
	let out = render(yaml);
	let expected = "# podup-owner: demo\n[Unit]\nDescription=web (podup)\n\n[Container]\nContainerName=demo-web\nImage=nginx\nTmpfs=/cache\nVolume=/srv/data:/data\nTmpfs=/scratch\nVolume=demo-data.volume:/seed\nVolume=/private\nLabel=podup.project=demo\nLabel=podup.service=web\nTmpfs=/run\nLogDriver=k8s-file\nLogOpt=max-size=10485760\n\n[Service]\nExecStartPre=-mkdir -p -- \"/srv/data\"\n\n[Install]\nWantedBy=default.target\n";
	assert_eq!(unit(&out, "demo-web.container").contents, expected);
}
