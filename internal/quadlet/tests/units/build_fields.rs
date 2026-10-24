//! The compose `build:` fields a `.build` unit carries.
//!
//! One test per field: the ones with a native Quadlet key land in the
//! `[Build]` section, the ones without one go through `PodmanArgs=` as the
//! `podman build` flag they stand for, and a field with neither is reported
//! instead of being dropped in silence.

use super::build_support::{build_unit, build_unit_for, warnings};

#[test]
fn build_pull_writes_the_pull_policy() {
	let unit = build_unit_for("pull: true");
	assert!(unit.contains("Pull=always"), "got:\n{unit}");
	// `pull: false` asks for the default policy, which is "pull the base
	// image only when it is missing", not "never".
	let unit = build_unit_for("pull: false");
	assert!(unit.contains("Pull=missing"), "got:\n{unit}");
}

#[test]
fn build_secrets_write_a_secret_key_per_top_level_secret() {
	let yaml = "services:\n  app:\n    build:\n      context: .\n      secrets:\n        - tok\n\
		secrets:\n  tok:\n    file: token.txt\n";
	let unit = build_unit(yaml);
	// `podman build --secret` reads a file, so the secret names its source
	// file; the path is absolute because the build runs with the context
	// directory as its working directory.
	assert!(
		unit.contains("Secret=id=tok,src=/srv/app/token.txt"),
		"got:\n{unit}"
	);
}

#[test]
fn build_platforms_write_the_architecture_and_the_variant() {
	let unit = build_unit_for("platforms:\n  - linux/arm64/v8");
	assert!(unit.contains("Arch=arm64"), "got:\n{unit}");
	assert!(unit.contains("Variant=v8"), "got:\n{unit}");
}

#[test]
fn build_cache_from_writes_a_cache_flag() {
	let unit = build_unit_for("cache_from:\n  - registry.example/cache:latest");
	assert!(
		unit.contains("PodmanArgs=--cache-from \"registry.example/cache:latest\""),
		"got:\n{unit}"
	);
	// Buildah ignores `--cache-from` unless the intermediate layers are
	// kept, so the flag that makes it work goes with it.
	assert!(unit.contains("PodmanArgs=--layers"), "got:\n{unit}");
}

#[test]
fn build_cache_to_writes_a_cache_flag() {
	let unit = build_unit_for("cache_to:\n  - registry.example/cache:latest");
	assert!(
		unit.contains("PodmanArgs=--cache-to \"registry.example/cache:latest\""),
		"got:\n{unit}"
	);
	assert!(unit.contains("PodmanArgs=--layers"), "got:\n{unit}");
}

#[test]
fn build_no_cache_writes_the_no_cache_flag() {
	let unit = build_unit_for("no_cache: true");
	assert!(unit.contains("PodmanArgs=--no-cache"), "got:\n{unit}");
}

#[test]
fn build_shm_size_writes_the_shm_size_flag() {
	let unit = build_unit_for("shm_size: 64m");
	assert!(
		unit.contains("PodmanArgs=--shm-size \"64m\""),
		"got:\n{unit}"
	);
}

#[test]
fn build_extra_hosts_write_an_add_host_flag() {
	let unit = build_unit_for("extra_hosts:\n  - \"db:10.0.0.2\"");
	assert!(
		unit.contains("PodmanArgs=--add-host \"db:10.0.0.2\""),
		"got:\n{unit}"
	);
}

#[test]
fn build_tags_write_a_tag_flag() {
	let unit = build_unit_for("tags:\n  - \"extra:1\"");
	assert!(
		unit.contains("PodmanArgs=--tag \"extra:1\""),
		"got:\n{unit}"
	);
}

#[test]
fn build_additional_contexts_write_a_build_context_flag() {
	let unit = build_unit_for("additional_contexts:\n  deps: ./deps");
	// The path is absolute: the build resolves a relative one against the
	// context directory, not against the compose file's directory.
	assert!(
		unit.contains("PodmanArgs=--build-context \"deps=/srv/app/deps\""),
		"got:\n{unit}"
	);
}

#[test]
fn build_ssh_writes_an_ssh_flag() {
	let unit = build_unit_for("ssh:\n  - default");
	assert!(
		unit.contains("PodmanArgs=--ssh \"default\""),
		"got:\n{unit}"
	);
}

#[test]
fn build_ulimits_write_an_ulimit_flag() {
	let unit = build_unit_for("ulimits:\n  nofile:\n    soft: 1024\n    hard: 2048");
	assert!(
		unit.contains("PodmanArgs=--ulimit \"nofile=1024:2048\""),
		"got:\n{unit}"
	);
}

#[test]
fn build_isolation_writes_an_isolation_flag() {
	let unit = build_unit_for("isolation: chroot");
	assert!(
		unit.contains("PodmanArgs=--isolation \"chroot\""),
		"got:\n{unit}"
	);
}

#[test]
fn build_privileged_is_reported_as_not_exported() {
	let yaml = "services:\n  app:\n    build:\n      context: .\n      privileged: true\n";
	assert!(
		warnings(yaml)
			.iter()
			.any(|w| w.contains("build.privileged")),
		"got:\n{:?}",
		warnings(yaml)
	);
}

#[test]
fn build_entitlements_are_reported_as_not_exported() {
	let yaml =
		"services:\n  app:\n    build:\n      context: .\n      entitlements:\n        - security.capability=NET_ADMIN\n";
	assert!(
		warnings(yaml)
			.iter()
			.any(|w| w.contains("build.entitlements")),
		"got:\n{:?}",
		warnings(yaml)
	);
}

#[test]
fn build_provenance_is_reported_as_not_exported() {
	let yaml = "services:\n  app:\n    build:\n      context: .\n      provenance: true\n";
	assert!(
		warnings(yaml)
			.iter()
			.any(|w| w.contains("build.provenance")),
		"got:\n{:?}",
		warnings(yaml)
	);
}

#[test]
fn build_sbom_is_reported_as_not_exported() {
	let yaml = "services:\n  app:\n    build:\n      context: .\n      sbom: true\n";
	assert!(
		warnings(yaml).iter().any(|w| w.contains("build.sbom")),
		"got:\n{:?}",
		warnings(yaml)
	);
}

/// An empty entry in a list field would render a bare `--flag=` podman
/// rejects with `error parsing additional short argument`, and the native
/// Quadlet keys drop empty values too. The non-empty entry still emits.
#[test]
fn an_empty_build_list_entry_emits_no_flag() {
	let unit = build_unit_for("cache_from:\n  - \"\"\n  - registry.example/cache:latest");
	assert!(
		!unit.contains("--cache-from \"\""),
		"an empty cache_from entry must not emit --cache-from=; got:\n{unit}"
	);
	assert!(
		unit.contains("PodmanArgs=--cache-from \"registry.example/cache:latest\""),
		"the non-empty entry must still emit; got:\n{unit}"
	);
}

/// An empty optional value emits no flag either.
#[test]
fn an_empty_build_scalar_emits_no_flag() {
	let unit = build_unit_for("shm_size: \"\"\nisolation: \"\"");
	assert!(
		!unit.contains("--shm-size"),
		"an empty shm_size must not emit --shm-size=; got:\n{unit}"
	);
	assert!(
		!unit.contains("--isolation"),
		"an empty isolation must not emit --isolation=; got:\n{unit}"
	);
}
