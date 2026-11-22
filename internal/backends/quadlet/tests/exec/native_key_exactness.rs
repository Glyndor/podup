//! A key Quadlet has natively is used only when the value reaches `podman`
//! with the bytes the compose file holds. Quadlet trims the whitespace around
//! the value of a key, and an assignment with nothing after the `=` empties a
//! list key, so a value that would be read back changed stays on
//! `PodmanArgs=`, which keeps every byte. These tests run the real generator
//! and read the argument it builds. They return early where it is not
//! installed.

#![cfg(unix)]

use super::native_key_values::{argv_of, generator, values_of};

/// Values with whitespace at an end: a space, and a no-break space, which
/// Quadlet trims as well.
const PADDED: &[&str] = &[" lead", "trail ", " both ", "\u{a0}nb\u{a0}"];

fn json(value: &str) -> String {
	serde_json::to_string(value).unwrap()
}

fn container(extra: &str) -> Vec<String> {
	let yaml = format!("services:\n  web:\n    image: docker.io/library/nginx\n{extra}");
	argv_of(&yaml, "container")
}

fn one_service_per_value(key: &str, values: &[&str]) -> String {
	let list: Vec<String> = values.iter().map(|v| json(v)).collect();
	format!("    {key}: [{}]\n", list.join(", "))
}

#[test]
fn apparmor_profile_keeps_its_boundary_spaces() {
	if generator().is_none() {
		return;
	}
	for value in PADDED {
		let argv = container(&format!(
			"    security_opt: [{}]\n",
			json(&format!("apparmor={value}"))
		));
		let got = values_of(&argv, "--security-opt");
		let want = format!("apparmor={value}");
		assert!(got.contains(&want), "{want:?} not in {got:?}");
	}
}

#[test]
fn list_and_scalar_container_keys_keep_their_boundary_spaces() {
	if generator().is_none() {
		return;
	}
	for value in PADDED {
		let argv = container(&format!(
			"{}{}    stop_signal: {}\n    logging:\n      driver: k8s-file\n      options:\n        k: {}\n",
			one_service_per_value("group_add", &[value]),
			one_service_per_value("extra_hosts", &[value]),
			json(value),
			json(value),
		));
		assert_eq!(values_of(&argv, "--group-add"), [*value], "{argv:?}");
		assert_eq!(values_of(&argv, "--add-host"), [*value], "{argv:?}");
		assert_eq!(values_of(&argv, "--stop-signal"), [*value], "{argv:?}");
		let want = format!("k={value}");
		assert_eq!(values_of(&argv, "--log-opt"), [want], "{argv:?}");
	}
}

/// Inside a pod whose services disagree on aliases, each container carries its
/// own, and that is where `NetworkAlias=` is written.
#[test]
fn network_alias_keeps_its_boundary_spaces() {
	if generator().is_none() {
		return;
	}
	for value in PADDED {
		let yaml = format!(
			"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    networks:\n      front:\n        aliases: [{v}]\n  db:\n    image: postgres\n    networks:\n      front:\n        aliases: [{v}, \"z\"]\nnetworks:\n  front:\n    enable_ipv6: true\n    ipam:\n      config:\n        - subnet: 10.5.0.0/24\n        - subnet: 2001:db8::/64\n",
			v = json(value)
		);
		let argv = argv_of(&yaml, "container");
		assert!(
			values_of(&argv, "--network-alias").contains(&value.to_string()),
			"{value:?} not in {argv:?}"
		);
	}
}

#[test]
fn build_values_keep_their_boundary_spaces() {
	if generator().is_none() {
		return;
	}
	for value in PADDED {
		let yaml = format!(
			"services:\n  app:\n    image: app:1.0\n    build:\n      context: .\n      args:\n        K: {}\n      tags: [{}]\n",
			json(value),
			json(value),
		);
		let argv = argv_of(&yaml, "build");
		let arg = format!("K={value}");
		assert_eq!(values_of(&argv, "--build-arg"), [arg], "{argv:?}");
		assert!(
			values_of(&argv, "--tag").contains(&value.to_string()),
			"{value:?} not in {argv:?}"
		);
	}
}

#[test]
fn pod_hosts_and_aliases_keep_their_boundary_spaces() {
	if generator().is_none() {
		return;
	}
	for value in PADDED {
		let hosts = format!("    extra_hosts: [{}]\n", json(value));
		let aliases = format!(
			"    networks:\n      front:\n        aliases: [{}]\n",
			json(value)
		);
		let yaml = format!(
			"x-podman-pod: true\nservices:\n  web:\n    image: nginx\n{hosts}{aliases}  db:\n    image: postgres\n{hosts}{aliases}networks:\n  front:\n    enable_ipv6: true\n    ipam:\n      config:\n        - subnet: 10.5.0.0/24\n        - subnet: 2001:db8::/64\n"
		);
		let argv = argv_of(&yaml, "pod");
		for flag in ["--add-host", "--network-alias"] {
			assert!(
				values_of(&argv, flag).contains(&value.to_string()),
				"{flag} {value:?} not in {argv:?}"
			);
		}
	}
}
/// An entry that is empty once its control characters are stripped must not
/// become a bare `Key=` line: Quadlet reads that as "empty the list" and the
/// entries before it are lost. Before the native keys, `--group-add=""` added
/// nothing and left the earlier groups alone.
#[test]
fn an_entry_that_strips_to_nothing_keeps_the_ones_before_it() {
	if generator().is_none() {
		return;
	}
	let argv = container(&format!(
		"{}{}",
		one_service_per_value("group_add", &["keep-groups", "\u{1}"]),
		one_service_per_value("extra_hosts", &["a:10.0.0.2", "\u{1}"]),
	));
	assert!(
		values_of(&argv, "--group-add").contains(&"keep-groups".to_string()),
		"{argv:?}"
	);
	assert!(
		values_of(&argv, "--add-host").contains(&"a:10.0.0.2".to_string()),
		"{argv:?}"
	);
}

#[test]
fn an_alias_that_strips_to_nothing_keeps_the_ones_before_it() {
	if generator().is_none() {
		return;
	}
	let yaml = "x-podman-pod: true\nservices:\n  web:\n    image: nginx\n    networks:\n      front:\n        aliases: [\"n1\", \"\\u0001\"]\n  db:\n    image: postgres\n    networks:\n      front:\n        aliases: [\"n1\", \"\\u0001\", \"z\"]\nnetworks:\n  front:\n    enable_ipv6: true\n    ipam:\n      config:\n        - subnet: 10.5.0.0/24\n        - subnet: 2001:db8::/64\n";
	let argv = argv_of(yaml, "container");
	assert!(
		values_of(&argv, "--network-alias").contains(&"n1".to_string()),
		"{argv:?}"
	);
}

#[test]
fn a_tag_that_strips_to_nothing_keeps_the_ones_before_it() {
	if generator().is_none() {
		return;
	}
	let yaml = format!(
		"services:\n  app:\n    image: app:1.0\n    build:\n      context: .\n      tags: [\"extra:1\", {}]\n",
		json("\u{1}")
	);
	let tags = values_of(&argv_of(&yaml, "build"), "--tag");
	assert!(tags.contains(&"app:1.0".to_string()), "{tags:?}");
	assert!(tags.contains(&"extra:1".to_string()), "{tags:?}");
}
