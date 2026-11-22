//! A value written to a native Quadlet key must reach `podman` as exactly one
//! argument, whatever it carries. The unit text does not show that: Quadlet
//! reads some keys as one string and others as shell-like words (`LogOpt=` and
//! `BuildArg=` split on whitespace and honour quote groups, without backslash
//! escapes), so these tests run the real generator over hostile values and
//! tokenise the `ExecStart=` line it builds. They return early where the
//! generator is not installed or is older than the floor (see `generator.rs`).

#![cfg(unix)]

use std::process::Command;

use super::podman_argv::tokenise_podman_argv;
use crate::backends::quadlet::tests::generator::generator;
use crate::parse_str;
use crate::quadlet::{generate_at, write_units};

/// The argv of the `podman run` / `podman build` line Quadlet builds for the
/// unit with this suffix, with systemd's `%%` read back as `%`.
pub(super) fn argv_of(yaml: &str, suffix: &str) -> Vec<String> {
	let generator = generator().expect("generator");
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	let dir = tempfile::tempdir().unwrap();
	write_units(dir.path(), &out.units).unwrap();
	let run = Command::new(generator)
		.args(["-dryrun", "-user"])
		.env("QUADLET_UNIT_DIRS", dir.path())
		.output()
		.unwrap();
	let text = String::from_utf8_lossy(&run.stdout).into_owned();
	let word = match suffix {
		"build" => " build ",
		"pod" => " pod create ",
		_ => " run ",
	};
	let line = text
		.lines()
		.find(|l| {
			(l.starts_with("ExecStart=/usr/bin/podman ")
				|| l.starts_with("ExecStartPre=/usr/bin/podman "))
				&& l.contains(word)
		})
		.unwrap_or_else(|| panic!("no ExecStart for {suffix}:\n{text}"));
	tokenise_podman_argv(line.split_once('=').expect("a directive").1)
		.into_iter()
		.map(|w| w.replace("%%", "%"))
		.collect()
}

/// The values of every occurrence of `flag` in `argv`, whether it was written
/// as `--flag value` (a native key) or as `--flag=value` (`PodmanArgs=`).
pub(super) fn values_of(argv: &[String], flag: &str) -> Vec<String> {
	let joined = format!("{flag}=");
	let mut out = Vec::new();
	for (i, word) in argv.iter().enumerate() {
		if word == flag {
			if let Some(next) = argv.get(i + 1) {
				out.push(next.clone());
			}
		} else if let Some(value) = word.strip_prefix(&joined) {
			out.push(value.to_string());
		}
	}
	out
}

const HOSTILE: &[&str] = &[
	"plain",
	"two words",
	"x --privileged -v /:/hostfs",
	"say \"hi\" now",
	"it's mine",
	"both \" and ' quotes",
	"back\\slash",
	"trailing\\",
	"100%h",
	"  padded  ",
];

fn yaml_list(values: &[&str]) -> String {
	serde_json::to_string(values).unwrap()
}

#[test]
fn log_opt_values_stay_one_argument_each() {
	if generator().is_none() {
		return;
	}
	let mut options = String::new();
	for (i, value) in HOSTILE.iter().enumerate() {
		options.push_str(&format!(
			"        k{i}: {}\n",
			serde_json::to_string(value).unwrap()
		));
	}
	let yaml = format!(
		"services:\n  web:\n    image: docker.io/library/nginx\n    logging:\n      driver: k8s-file\n      options:\n{options}"
	);
	let argv = argv_of(&yaml, "container");
	let got = values_of(&argv, "--log-opt");
	for (i, value) in HOSTILE.iter().enumerate() {
		let want = format!("k{i}={value}");
		assert!(got.contains(&want), "missing {want:?} in {got:?}");
	}
	assert!(!argv.iter().any(|a| a == "--privileged"), "{argv:?}");
	assert_eq!(got.len(), HOSTILE.len(), "{got:?}");
}

#[test]
fn build_arg_values_stay_one_argument_each() {
	if generator().is_none() {
		return;
	}
	let mut args = String::new();
	for (i, value) in HOSTILE.iter().enumerate() {
		args.push_str(&format!(
			"        K{i}: {}\n",
			serde_json::to_string(value).unwrap()
		));
	}
	let yaml = format!(
		"services:\n  app:\n    image: app:1.0\n    build:\n      context: .\n      args:\n{args}"
	);
	let argv = argv_of(&yaml, "build");
	let got = values_of(&argv, "--build-arg");
	for (i, value) in HOSTILE.iter().enumerate() {
		let want = format!("K{i}={value}");
		assert!(got.contains(&want), "missing {want:?} in {got:?}");
	}
	assert!(!argv.iter().any(|a| a == "--privileged"), "{argv:?}");
	assert_eq!(got.len(), HOSTILE.len(), "{got:?}");
}

#[test]
fn single_string_keys_stay_one_argument_each() {
	if generator().is_none() {
		return;
	}
	let hostile = ["x --privileged -v /:/hostfs", "say \"hi\" 'there'"];
	let yaml = format!(
		"services:\n  web:\n    image: docker.io/library/nginx\n    group_add: {}\n    extra_hosts: {}\n    stop_signal: {}\n",
		yaml_list(&hostile),
		yaml_list(&["h --privileged:10.0.0.2"]),
		serde_json::to_string("SIG --privileged").unwrap(),
	);
	let argv = argv_of(&yaml, "container");
	assert_eq!(values_of(&argv, "--group-add"), hostile, "{argv:?}");
	assert_eq!(
		values_of(&argv, "--add-host"),
		["h --privileged:10.0.0.2"],
		"{argv:?}"
	);
	assert_eq!(
		values_of(&argv, "--stop-signal"),
		["SIG --privileged"],
		"{argv:?}"
	);
	assert!(!argv.iter().any(|a| a == "--privileged"), "{argv:?}");
}

#[test]
fn extra_image_tags_stay_one_argument_each() {
	if generator().is_none() {
		return;
	}
	let yaml = format!(
		"services:\n  app:\n    image: app:1.0\n    build:\n      context: .\n      tags: {}\n",
		yaml_list(&["extra:1", "x --privileged y"]),
	);
	let argv = argv_of(&yaml, "build");
	let tags = values_of(&argv, "--tag");
	assert!(tags.contains(&"app:1.0".to_string()), "{tags:?}");
	assert!(tags.contains(&"extra:1".to_string()), "{tags:?}");
	assert!(tags.contains(&"x --privileged y".to_string()), "{tags:?}");
	assert!(!argv.iter().any(|a| a == "--privileged"), "{argv:?}");
}

#[test]
fn an_exec_form_healthcheck_reaches_podman_as_a_json_array() {
	if generator().is_none() {
		return;
	}
	let words = [
		"CMD",
		"sh",
		"-c",
		"wget -q 'x' \"y\" \\ || exit 1",
		"100%h",
		"two  spaces",
	];
	let yaml = format!(
		"services:\n  web:\n    image: docker.io/library/nginx\n    healthcheck:\n      test: {}\n",
		yaml_list(&words),
	);
	let argv = argv_of(&yaml, "container");
	let got = values_of(&argv, "--health-cmd");
	assert_eq!(got.len(), 1, "{argv:?}");
	let parsed: Vec<String> = serde_json::from_str(&got[0]).expect("a JSON array");
	assert_eq!(parsed, words, "{argv:?}");
}
