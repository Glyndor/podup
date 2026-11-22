//! A hostile value written to a native Quadlet key must reach `podman` as one
//! argument. Each case checks the unit text, which needs no Podman (the whole
//! value sits on one native line and no `PodmanArgs=` line carries it), and,
//! where the generator is installed, the argument it builds from that line.

#![cfg(unix)]

use super::native_key_values::{argv_of, values_of};
use crate::backends::quadlet::tests::generator::generator;
use crate::parse_str;
use crate::quadlet::generate_at;
use crate::quadlet::tests::unit_named;

fn unit_text(yaml: &str, unit: &str) -> String {
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "proj", std::path::Path::new("/srv/app"));
	unit_named(&out, unit).contents.clone()
}

/// No `PodmanArgs=` line of the unit carries `needle`.
fn assert_not_in_podman_args(text: &str, needle: &str) {
	for line in text.lines().filter(|l| l.starts_with("PodmanArgs=")) {
		assert!(!line.contains(needle), "{needle:?} in {line:?}:\n{text}");
	}
}

/// The generator's argument for `flag` is exactly `want`, and the hostile
/// words are not arguments of their own.
fn assert_generated_argument(yaml: &str, suffix: &str, flag: &str, want: &str, hostile: &[&str]) {
	if generator().is_none() {
		return;
	}
	let argv = argv_of(yaml, suffix);
	assert_eq!(values_of(&argv, flag), [want], "{argv:?}");
	for word in hostile {
		assert!(!argv.iter().any(|a| a == word), "{word:?} in {argv:?}");
	}
}

const HOSTILE_WORDS: &[&str] = &["--privileged", "-v"];

#[test]
fn apparmor_profile_stays_one_security_opt_argument() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    security_opt:\n      - \"apparmor=my-profile --privileged -v /:/hostfs\"\n";
	let text = unit_text(yaml, "proj-web.container");
	assert!(
		text.contains("\nAppArmor=my-profile --privileged -v /:/hostfs\n"),
		"{text}"
	);
	assert_not_in_podman_args(&text, "--privileged");
	assert_generated_argument(
		yaml,
		"container",
		"--security-opt",
		"apparmor=my-profile --privileged -v /:/hostfs",
		HOSTILE_WORDS,
	);
}

#[test]
fn mem_limit_stays_one_memory_argument() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    mem_limit: \"512m --privileged -v /:/hostfs\"\n";
	let text = unit_text(yaml, "proj-web.container");
	assert!(
		text.contains("\nMemory=512m --privileged -v /:/hostfs\n"),
		"{text}"
	);
	assert_not_in_podman_args(&text, "--privileged");
	assert_generated_argument(
		yaml,
		"container",
		"--memory",
		"512m --privileged -v /:/hostfs",
		HOSTILE_WORDS,
	);
}

#[test]
fn deploy_memory_stays_one_memory_argument() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    deploy:\n      resources:\n        limits:\n          memory: \"256m --privileged\"\n";
	let text = unit_text(yaml, "proj-web.container");
	assert!(text.contains("\nMemory=256m --privileged\n"), "{text}");
	assert_not_in_podman_args(&text, "--privileged");
	assert_generated_argument(
		yaml,
		"container",
		"--memory",
		"256m --privileged",
		HOSTILE_WORDS,
	);
}

#[test]
fn a_build_arg_value_stays_one_argument() {
	let yaml = "services:\n  app:\n    image: app:1.0\n    build:\n      context: .\n      args:\n        EVIL: \"v --privileged -v /:/hostfs\"\n";
	let text = unit_text(yaml, "proj-app.build");
	assert!(
		text.contains("\nBuildArg=\"EVIL=v --privileged -v /:/hostfs\"\n"),
		"{text}"
	);
	assert_not_in_podman_args(&text, "--privileged");
	assert_generated_argument(
		yaml,
		"build",
		"--build-arg",
		"EVIL=v --privileged -v /:/hostfs",
		HOSTILE_WORDS,
	);
}

#[test]
fn a_bare_build_arg_key_stays_one_argument() {
	let yaml = "services:\n  app:\n    image: app:1.0\n    build:\n      context: .\n      args:\n        - \"BAD --privileged\"\n";
	let text = unit_text(yaml, "proj-app.build");
	assert!(text.contains("\nBuildArg=\"BAD --privileged\"\n"), "{text}");
	assert_not_in_podman_args(&text, "--privileged");
	assert_generated_argument(
		yaml,
		"build",
		"--build-arg",
		"BAD --privileged",
		HOSTILE_WORDS,
	);
}

/// A systemd specifier in the value is doubled, so Podman receives `%h`
/// literally instead of the host name.
#[test]
fn a_specifier_in_a_native_value_reaches_podman_literally() {
	let yaml = "services:\n  web:\n    image: alpine:3.20\n    mem_limit: \"%h/mem\"\n";
	let text = unit_text(yaml, "proj-web.container");
	assert!(text.contains("\nMemory=%%h/mem\n"), "{text}");
	assert_generated_argument(yaml, "container", "--memory", "%h/mem", &[]);
}
