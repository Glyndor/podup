//! The compose `build:` values the export has to guard.
//!
//! A value that reaches `podman build` unchanged can break the build: an
//! `--isolation` naming a compose isolation technology, a `Secret=` whose
//! path holds a comma, an `additional_contexts` entry that names a service.
//! Each of those is reported instead of being exported, and the values that
//! can be carried are carried whole (a secret path holding a space, a URL
//! context).

use super::build_support::{build_unit, build_unit_at, build_unit_for, warnings};

/// Quadlet takes the `Secret=` value apart with an exec-like split, so a
/// path holding a space would arrive at `podman` as two `--secret` flags,
/// each one broken. The value is quoted, and the quotes are what keep it one
/// value.
#[test]
#[cfg(unix)]
fn a_secret_whose_path_holds_a_space_stays_one_value() {
	let yaml = "services:\n  app:\n    build:\n      context: .\n      secrets:\n        - tok\n\
		secrets:\n  tok:\n    file: token.txt\n";
	let unit = build_unit_at(yaml, "/srv/My Projects");
	assert!(
		unit.contains("Secret=\"id=tok,src=/srv/My Projects/token.txt\""),
		"the secret must stay one quoted value; got:\n{unit}"
	);
}

/// `Secret=` is a comma-separated option list, so a comma in the path would
/// break the value into options the build cannot read.
#[test]
fn a_secret_whose_path_holds_a_comma_is_reported_and_skipped() {
	let yaml = "services:\n  app:\n    build:\n      context: .\n      secrets:\n        - tok\n\
		secrets:\n  tok:\n    file: \"to,ken.txt\"\n";
	let unit = build_unit(yaml);
	assert!(
		!unit.contains("Secret="),
		"a secret whose path holds a comma must not be exported; got:\n{unit}"
	);
	let joined = warnings(yaml).join("\n");
	assert!(
		joined.contains("tok") && joined.contains("comma"),
		"expected a warning naming the secret and the comma; got:\n{joined}"
	);
}

/// The same break, on the id side of the option list.
#[test]
fn a_secret_whose_id_holds_a_comma_is_reported_and_skipped() {
	let yaml =
		"services:\n  app:\n    build:\n      context: .\n      secrets:\n        - \"to,ken\"\n\
		secrets:\n  \"to,ken\":\n    file: token.txt\n";
	let unit = build_unit(yaml);
	assert!(
		!unit.contains("Secret="),
		"a secret whose id holds a comma must not be exported; got:\n{unit}"
	);
	let joined = warnings(yaml).join("\n");
	assert!(
		joined.contains("comma"),
		"expected a warning naming the comma; got:\n{joined}"
	);
}

/// A `build.secrets` entry the top-level `secrets:` section does not define
/// has no file to point at, so there is nothing to export.
#[test]
fn a_secret_the_compose_file_does_not_define_is_reported() {
	let yaml =
		"services:\n  app:\n    build:\n      context: .\n      secrets:\n        - missing\n";
	let unit = build_unit(yaml);
	assert!(
		!unit.contains("Secret="),
		"an undefined secret must not be exported; got:\n{unit}"
	);
	let joined = warnings(yaml).join("\n");
	assert!(
		joined.contains("missing"),
		"expected a warning naming the secret; got:\n{joined}"
	);
}

/// `podman build --secret` reads a file, so a secret with an inline source
/// has no path to give it.
#[test]
fn a_secret_without_a_file_source_is_reported() {
	let yaml = "services:\n  app:\n    build:\n      context: .\n      secrets:\n        - tok\n\
		secrets:\n  tok:\n    environment: TOKEN\n";
	let unit = build_unit(yaml);
	assert!(
		!unit.contains("Secret="),
		"a secret with no file source must not be exported; got:\n{unit}"
	);
	let joined = warnings(yaml).join("\n");
	assert!(
		joined.contains("tok") && joined.contains("file"),
		"expected a warning naming the secret and the missing file; got:\n{joined}"
	);
}

/// Compose's `isolation` names a container isolation technology (`default`,
/// `process`, `hyperv`), which buildah refuses. Passing it through would
/// make the build fail, where before the field was carried the build ran.
#[test]
fn an_isolation_that_is_not_a_buildah_type_is_reported() {
	let unit = build_unit_for("isolation: default");
	assert!(
		!unit.contains("--isolation"),
		"a compose isolation name must not reach `podman build`; got:\n{unit}"
	);
	let joined =
		warnings("services:\n  app:\n    build:\n      context: .\n      isolation: default\n")
			.join("\n");
	assert!(
		joined.contains("build.isolation") && joined.contains("not exported"),
		"expected a warning naming the field; got:\n{joined}"
	);
}

/// An `additional_contexts` entry that names a service is not a path and not
/// a URL, so making it absolute would point the build at a directory that
/// does not exist.
#[test]
fn an_additional_context_naming_a_service_is_reported_and_skipped() {
	let unit = build_unit_for("additional_contexts:\n  other: service:other");
	assert!(
		!unit.contains("--build-context"),
		"a context naming a service must not be exported; got:\n{unit}"
	);
	let joined = warnings(
		"services:\n  app:\n    build:\n      context: .\n      additional_contexts:\n        other: service:other\n",
	)
	.join("\n");
	assert!(
		joined.contains("build.additional_contexts") && joined.contains("not exported"),
		"expected a warning naming the field; got:\n{joined}"
	);
}

/// A context that is a URL stays the URL it is: it is not a path to make
/// absolute.
#[test]
fn an_additional_context_that_is_a_url_is_passed_through() {
	let unit = build_unit_for("additional_contexts:\n  base: docker-image://alpine");
	assert!(
		unit.contains("PodmanArgs=--build-context \"base=docker-image://alpine\""),
		"a URL context must be passed through as it is; got:\n{unit}"
	);
}

/// The section carries one `Arch=`/`Variant=` pair, so a compose file that
/// lists several platforms has only the first one exported.
#[test]
fn several_platforms_are_reported() {
	let unit = build_unit_for("platforms:\n  - linux/arm64\n  - linux/amd64");
	assert!(
		unit.contains("Arch=arm64"),
		"the first platform is the exported one; got:\n{unit}"
	);
	assert!(
		!unit.contains("Arch=amd64"),
		"only the first platform can be exported; got:\n{unit}"
	);
	let joined = warnings(
		"services:\n  app:\n    build:\n      context: .\n      platforms:\n        - linux/arm64\n        - linux/amd64\n",
	)
	.join("\n");
	assert!(
		joined.contains("build.platforms"),
		"expected a warning naming the field; got:\n{joined}"
	);
}

/// The section has no `OS=` key, so a platform naming another OS is built
/// with the host's OS instead.
#[test]
fn a_platform_naming_another_os_is_reported() {
	let unit = build_unit_for("platforms:\n  - windows/amd64");
	assert!(
		unit.contains("Arch=amd64"),
		"the architecture is still exported; got:\n{unit}"
	);
	let joined = warnings(
		"services:\n  app:\n    build:\n      context: .\n      platforms:\n        - windows/amd64\n",
	)
	.join("\n");
	assert!(
		joined.contains("windows"),
		"expected a warning naming the OS; got:\n{joined}"
	);
}

/// The ulimit name reaches a Podman flag, so a name outside the set of Linux
/// rlimit resources is either a typo or an injection attempt.
#[test]
fn an_unknown_ulimit_name_is_reported_and_skipped() {
	let unit = build_unit_for("ulimits:\n  nofille:\n    soft: 1024\n    hard: 2048");
	assert!(
		!unit.contains("--ulimit"),
		"an unknown rlimit name must not be exported; got:\n{unit}"
	);
	let joined = warnings(
		"services:\n  app:\n    build:\n      context: .\n      ulimits:\n        nofille:\n          soft: 1024\n          hard: 2048\n",
	)
	.join("\n");
	assert!(
		joined.contains("nofille"),
		"expected a warning naming the resource; got:\n{joined}"
	);
}
