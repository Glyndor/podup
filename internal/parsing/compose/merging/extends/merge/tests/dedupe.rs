/// Merge two files and return the service `s`.
fn merged(base: &str, over: &str) -> crate::compose::types::Service {
	let dir = tempfile::tempdir().unwrap();
	let a = dir.path().join("a.yaml");
	let b = dir.path().join("b.yaml");
	std::fs::write(&a, base).unwrap();
	std::fs::write(&b, over).unwrap();
	crate::parsing::compose::parse_files_with_env_files(&[a, b], &[])
		.unwrap()
		.services
		.shift_remove("s")
		.unwrap()
}

#[test]
fn a_port_written_with_and_without_tcp_is_one_mapping() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    ports: ['127.0.0.1:8080:80']\n",
		"services:\n  s:\n    ports: ['127.0.0.1:8080:80/tcp']\n",
	);
	assert_eq!(svc.ports.len(), 1);
}

#[test]
fn tcp_and_udp_of_the_same_port_stay_two_mappings() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    ports: ['5353:53']\n",
		"services:\n  s:\n    ports: ['5353:53/udp']\n",
	);
	assert_eq!(svc.ports.len(), 2);
}

#[test]
fn a_trailing_slash_does_not_make_a_second_mount_at_the_same_path() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    volumes: ['./d:/data']\n",
		"services:\n  s:\n    volumes: ['./e:/data/']\n",
	);
	assert_eq!(svc.volumes.len(), 1);
	assert!(svc.volumes[0].target().starts_with("/data"));
}

#[test]
fn different_mount_paths_are_both_kept() {
	let svc = merged(
		"services:\n  s:\n    image: x\n    volumes: ['./d:/data']\n",
		"services:\n  s:\n    volumes: ['./e:/data2']\n",
	);
	assert_eq!(svc.volumes.len(), 2);
}
