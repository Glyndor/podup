use super::*;

fn devices(yaml: &str) -> Result<Vec<String>, serde_yaml::Error> {
	#[derive(Deserialize)]
	struct Holder {
		#[serde(deserialize_with = "deserialize")]
		devices: Vec<String>,
	}
	serde_yaml::from_str::<Holder>(yaml).map(|h| h.devices)
}

#[test]
fn long_entries_become_the_short_form() {
	let list = devices(
		"devices:\n  - {source: /dev/null, target: /dev/n, permissions: rwm}\n  - {source: /dev/zero, target: /dev/z}\n",
	)
	.unwrap();
	assert_eq!(list, ["/dev/null:/dev/n:rwm", "/dev/zero:/dev/z"]);
}

#[test]
fn short_and_long_entries_mix() {
	let list =
		devices("devices:\n  - /dev/null\n  - {source: /dev/zero, target: /dev/z}\n").unwrap();
	assert_eq!(list, ["/dev/null", "/dev/zero:/dev/z"]);
}

#[test]
fn a_long_entry_without_a_target_is_refused_by_index() {
	let err = devices("devices:\n  - /dev/null\n  - {source: /dev/zero}\n")
		.unwrap_err()
		.to_string();
	assert!(
		err.contains("devices[1] is missing a mount target"),
		"{err}"
	);
}

#[test]
fn a_long_entry_without_a_source_or_with_an_unknown_key_is_refused() {
	assert!(devices("devices:\n  - {target: /dev/z}\n").is_err());
	assert!(devices("devices:\n  - {source: /dev/a, target: /dev/z, mode: rw}\n").is_err());
}

#[test]
fn the_container_path_is_the_second_segment_or_the_host_path() {
	assert_eq!(container_path("/dev/null"), "/dev/null");
	assert_eq!(container_path("/dev/null:/dev/n"), "/dev/n");
	assert_eq!(container_path("/dev/null:/dev/n:rwm"), "/dev/n");
	assert_eq!(container_path("/dev/null::rwm"), "/dev/null");
}
