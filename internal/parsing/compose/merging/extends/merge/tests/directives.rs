use super::*;

/// #1078: `!override` takes the overriding value whole instead of appending.
/// Both tags were accepted and silently ignored, so a file asking for
/// replacement got a merge, the opposite of what it said.
#[test]
fn override_tag_replaces_instead_of_appending() {
	let base =
		parse_str("services:\n  web:\n    image: alpine\n    ports: [\"8080:80\"]\n").unwrap();
	let over = parse_str("services:\n  web:\n    ports: !override [\"9090:80\"]\n").unwrap();
	let raw: serde_yaml::Value =
		serde_yaml::from_str("services:\n  web:\n    ports: !override [\"9090:80\"]\n").unwrap();
	let directives = crate::compose::tags::collect(&raw);

	let merged = crate::compose::extends::merge_service_tagged(
		base.services["web"].clone(),
		over.services["web"].clone(),
		directives.get("web"),
	);
	assert_eq!(
		merged.ports.len(),
		1,
		"!override must replace, not append: {:?}",
		merged.ports
	);
}

/// `!reset` drops the key entirely, leaving the type's default.
#[test]
fn reset_tag_clears_the_base_value() {
	let base = parse_str("services:\n  web:\n    image: alpine\n    dns: [\"1.1.1.1\"]\n").unwrap();
	let over = parse_str("services:\n  web:\n    dns: !reset []\n").unwrap();
	let raw: serde_yaml::Value =
		serde_yaml::from_str("services:\n  web:\n    dns: !reset []\n").unwrap();
	let directives = crate::compose::tags::collect(&raw);

	let merged = crate::compose::extends::merge_service_tagged(
		base.services["web"].clone(),
		over.services["web"].clone(),
		directives.get("web"),
	);
	assert!(
		merged.dns.to_list().is_empty(),
		"!reset must clear the base: {:?}",
		merged.dns
	);
}

/// Without a tag the ordinary merge rule still applies; the tags must not
/// change the default behaviour of anything they are not on.
#[test]
fn an_untagged_key_still_merges_normally() {
	let base = parse_str("services:\n  web:\n    image: alpine\n    dns: [\"1.1.1.1\"]\n").unwrap();
	let over = parse_str("services:\n  web:\n    dns: [\"9.9.9.9\"]\n").unwrap();
	let merged = crate::compose::extends::merge_service_tagged(
		base.services["web"].clone(),
		over.services["web"].clone(),
		None,
	);
	assert_eq!(merged.dns.to_list(), vec!["1.1.1.1", "9.9.9.9"]);
}

fn merged_with_tags(base: &str, over: &str) -> crate::compose::types::Service {
	let base = parse_str(base).unwrap();
	let over_file = parse_str(over).unwrap();
	let raw: serde_yaml::Value = serde_yaml::from_str(over).unwrap();
	let directives = crate::compose::tags::collect(&raw);
	crate::compose::extends::merge_service_tagged(
		base.services["web"].clone(),
		over_file.services["web"].clone(),
		directives.get("web"),
	)
}

/// The blocks that merge key by key still honour the tags: `!override` takes
/// the new block whole and `!reset` empties it.
#[test]
fn override_tag_replaces_a_block_that_merges_key_by_key() {
	let merged = merged_with_tags(
		"services:\n  web:\n    build:\n      context: ./app\n      args:\n        MODE: debug\n",
		"services:\n  web:\n    build: !override\n      context: ./other\n",
	);
	let build = merged.build.expect("build");
	assert_eq!(build.context(), "./other");
	assert!(build.args().is_empty(), "the base args survived !override");
}

#[test]
fn reset_tag_empties_a_block_that_merges_key_by_key() {
	let merged = merged_with_tags(
		"services:\n  web:\n    image: alpine\n    healthcheck:\n      test: [\"CMD\", \"true\"]\n",
		"services:\n  web:\n    healthcheck: !reset {}\n",
	);
	assert!(
		merged.healthcheck.is_none(),
		"the base healthcheck survived !reset"
	);
}

/// Docker Compose 5.7.0 honours `!reset` on every service key. A key missing
/// from the reset list kept the earlier file's value instead; each of these did.
#[test]
fn reset_tag_clears_every_scalar_service_key() {
	let keys = [
		("hostname", "h"),
		("domainname", "d.example"),
		("user", "\"1\""),
		("working_dir", "/w"),
		("container_name", "cn"),
		("restart", "always"),
		("platform", "linux/amd64"),
		("stop_signal", "SIGINT"),
		("stop_grace_period", "5s"),
		("mac_address", "02:42:ac:11:00:02"),
		("shm_size", "64m"),
		("pids_limit", "10"),
		("mem_limit", "100m"),
		("init", "true"),
		("read_only", "true"),
		("privileged", "true"),
		("tty", "true"),
		("stdin_open", "true"),
		("pull_policy", "always"),
		("network_mode", "host"),
	];
	let mut base = String::from("services:\n  web:\n    image: alpine\n");
	let mut over = String::from("services:\n  web:\n");
	for (key, value) in keys {
		base.push_str(&format!("    {key}: {value}\n"));
		over.push_str(&format!("    {key}: !reset null\n"));
	}
	let merged = merged_with_tags(&base, &over);
	let value = serde_yaml::to_value(&merged).unwrap();
	for (key, _) in keys {
		assert!(
			value.get(key).is_none(),
			"!reset must clear {key}: {:?}",
			value.get(key)
		);
	}
}

/// `!override` on these keys used to combine both files (append or merge by key)
/// because they were not in the reset list; Docker keeps only the later value.
#[test]
fn override_tag_replaces_the_keys_that_used_to_combine() {
	let merged = merged_with_tags(
		"services:\n  web:\n    image: alpine\n    annotations: {a: \"1\"}\n    \
		 device_cgroup_rules: [\"c 1:3 mr\"]\n    post_start: [{command: [a]}]\n",
		"services:\n  web:\n    annotations: !override {b: \"2\"}\n    \
		 device_cgroup_rules: !override [\"c 1:5 mr\"]\n    \
		 post_start: !override [{command: [b]}]\n",
	);
	let value = serde_yaml::to_value(&merged).unwrap();
	assert_eq!(
		value["annotations"],
		serde_yaml::from_str::<serde_yaml::Value>("{b: \"2\"}").unwrap()
	);
	assert_eq!(
		value["device_cgroup_rules"],
		serde_yaml::from_str::<serde_yaml::Value>("[\"c 1:5 mr\"]").unwrap()
	);
	assert_eq!(
		value["post_start"].as_sequence().map(Vec::len),
		Some(1),
		"{value:?}"
	);
}
