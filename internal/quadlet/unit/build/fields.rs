//! The compose `build:` fields a `.build` unit carries.
//!
//! Quadlet's `[Build]` section has a native key for some of the compose
//! build fields (`Pull=`, `Secret=`, `Arch=`, `Variant=`) and none for the
//! rest, which go through `PodmanArgs=` as the `podman build` flag they
//! stand for. A field with neither a key nor a flag is reported by
//! `collect_build_warnings` instead of being dropped.
//!
//! Every key this module writes natively landed with the `.build` unit
//! type itself (Podman 5.2.0), so a 5.2 host loads the unit. The keys that
//! post-date it (`BuildArg=` in 5.7.0) stay behind `PodmanArgs=`, which
//! every unit type has carried since 5.0.

use std::collections::HashMap;
use std::path::Path;

use indexmap::IndexMap;

use crate::compose::types::{BuildConfig, SecretConfig, UlimitConfig};

use super::super::{abs_against, quote_podman_arg_value, Section};

/// Emit every compose `build:` field that has a `.build` key or a Podman
/// flag, in the order the `[Build]` section lists them.
pub(super) fn emit_build_fields(
	name: &str,
	build: &BuildConfig,
	base_dir: &Path,
	secrets: &IndexMap<String, SecretConfig>,
	section: &mut Section,
	warnings: &mut Vec<String>,
) {
	let BuildConfig::Config {
		pull,
		secrets: secret_names,
		platforms,
		cache_from,
		cache_to,
		shm_size,
		no_cache,
		extra_hosts,
		tags,
		additional_contexts,
		ssh,
		ulimits,
		isolation,
		..
	} = build
	else {
		return;
	};

	emit_pull(*pull, section);
	emit_secrets(name, secret_names, secrets, base_dir, section, warnings);
	emit_platforms(name, platforms, section, warnings);
	emit_cache(cache_from, cache_to, section);
	if *no_cache == Some(true) {
		section.add("PodmanArgs", "--no-cache".to_string());
	}
	if let Some(size) = shm_size.as_deref() {
		add_flag(section, "--shm-size", size);
	}
	for host in extra_hosts {
		add_flag(section, "--add-host", host);
	}
	for tag in tags {
		add_flag(section, "--tag", tag);
	}
	emit_additional_contexts(name, additional_contexts, base_dir, section, warnings);
	for entry in ssh {
		add_flag(section, "--ssh", entry);
	}
	emit_ulimits(name, ulimits, section, warnings);
	emit_isolation(name, isolation.as_deref(), section, warnings);
}

/// Add one `PodmanArgs=` line carrying a Podman flag and its value.
///
/// The value is quoted the way the other `PodmanArgs=` sites quote theirs,
/// so a hostile compose value stays one argv element instead of becoming a
/// second Podman flag. An empty value emits nothing: podman rejects a bare
/// `--flag=` with `error parsing additional short argument`, and the native
/// Quadlet keys drop empty values too.
fn add_flag(section: &mut Section, flag: &str, value: &str) {
	if value.is_empty() {
		return;
	}
	section.add(
		"PodmanArgs",
		format!("{flag} {}", quote_podman_arg_value(value)),
	);
}

/// Emit the `Pull=` key for `build.pull`.
///
/// Compose's `pull` is a boolean: true asks for a newer base image on every
/// build, false for the default policy, which pulls the base image only
/// when it is missing. The `[Build]` key takes the policy name
/// `podman build --pull` takes.
fn emit_pull(pull: Option<bool>, section: &mut Section) {
	match pull {
		Some(true) => section.add("Pull", "always".to_string()),
		Some(false) => section.add("Pull", "missing".to_string()),
		None => {}
	}
}

/// Emit one `Secret=` line per `build.secrets` entry.
///
/// `podman build --secret` reads the value from a file, not from the
/// engine's secret store, so a top-level secret with a `file:` source
/// becomes `id=<name>,src=<absolute path>`; the path is absolute because
/// the build resolves a relative one against the context directory rather
/// than against the compose file's directory. A secret with any other
/// source (inline `content:`, `environment:`, `external:`) has no file to
/// point at and is reported instead.
///
/// The value is a comma-separated option list, so a comma in the id or in
/// the path would break it apart into options the build cannot read. Such a
/// secret is reported and left out rather than exported as a value the
/// build would refuse.
fn emit_secrets(
	name: &str,
	secret_names: &[String],
	secrets: &IndexMap<String, SecretConfig>,
	base_dir: &Path,
	section: &mut Section,
	warnings: &mut Vec<String>,
) {
	for secret_name in secret_names {
		let Some(def) = secrets.get(secret_name) else {
			warnings.push(format!(
				"{name}: build.secrets names {secret_name:?}, which the top-level secrets section \
				 does not define; it is skipped"
			));
			continue;
		};
		let Some(file) = def.file.as_deref() else {
			warnings.push(format!(
				"{name}: build secret {secret_name:?} has no file source; \
				 `podman build --secret` reads a file, so it is skipped"
			));
			continue;
		};
		let src = abs_against(base_dir, file);
		if secret_name.contains(',') || src.contains(',') {
			warnings.push(format!(
				"{name}: build secret {secret_name:?} is skipped: `Secret=` is a comma-separated \
				 option list, so a comma in its id or in its path would break it apart"
			));
			continue;
		}
		section.add("Secret", format!("id={secret_name},src={src}"));
	}
}

/// Emit the `Arch=` and `Variant=` keys for the first `build.platforms`
/// entry.
///
/// The `[Build]` section carries one architecture and one variant, so a
/// compose file listing several platforms has only the first one exported;
/// the rest are reported. The OS part of a platform string has no key at
/// all (the section has no `OS=`), so a platform naming an OS other than
/// the host's is reported too.
fn emit_platforms(
	name: &str,
	platforms: &[String],
	section: &mut Section,
	warnings: &mut Vec<String>,
) {
	let Some(first) = platforms.first() else {
		return;
	};
	if platforms.len() > 1 {
		warnings.push(format!(
			"{name}: build.platforms lists {} platforms; the .build unit carries one \
			 Arch=/Variant= pair, so only {first} is exported",
			platforms.len()
		));
	}
	let (os, arch, variant) = split_platform(first);
	section.add("Arch", arch.to_string());
	if let Some(variant) = variant.filter(|v| !v.is_empty()) {
		section.add("Variant", variant.to_string());
	}
	if !os.is_empty() && os != "linux" {
		warnings.push(format!(
			"{name}: build.platforms names the OS {os:?}, which the .build unit cannot carry \
			 (it has no OS= key); the build uses the host OS"
		));
	}
}

/// Split a compose platform string into its OS, architecture and variant.
///
/// The compose form is `os/arch[/variant]`; a bare architecture is
/// accepted too.
fn split_platform(platform: &str) -> (&str, &str, Option<&str>) {
	let mut parts = platform.split('/');
	let first = parts.next().unwrap_or_default();
	match parts.next() {
		Some(arch) => (first, arch, parts.next()),
		None => ("", first, None),
	}
}

/// Emit the cache flags for `build.cache_from` and `build.cache_to`.
///
/// Buildah ignores `--cache-from` and `--cache-to` unless the intermediate
/// layers are kept, so the flag that makes them work goes with them.
fn emit_cache(cache_from: &[String], cache_to: &[String], section: &mut Section) {
	let any_source = cache_from.iter().any(|source| !source.is_empty());
	let any_destination = cache_to.iter().any(|destination| !destination.is_empty());
	if !any_source && !any_destination {
		return;
	}
	section.add("PodmanArgs", "--layers".to_string());
	for source in cache_from {
		add_flag(section, "--cache-from", source);
	}
	for destination in cache_to {
		add_flag(section, "--cache-to", destination);
	}
}

/// Emit one `--build-context` flag per `build.additional_contexts` entry.
///
/// A local path is made absolute against the compose file's directory: the
/// build runs with the context directory as its working directory, so a
/// relative path would resolve somewhere else. A URL is passed through.
///
/// A value that names a source instead of pointing at one (compose's
/// `service:other`, which means "the image the service `other` builds") is
/// reported and left out: `podman build --build-context` takes a path or a
/// URL, and making the name absolute would point the build at a directory
/// that does not exist.
fn emit_additional_contexts(
	name: &str,
	contexts: &HashMap<String, String>,
	base_dir: &Path,
	section: &mut Section,
	warnings: &mut Vec<String>,
) {
	let mut sorted: Vec<(&String, &String)> = contexts.iter().collect();
	sorted.sort_by(|a, b| a.0.cmp(b.0));
	for (context_name, value) in sorted {
		if value.is_empty() {
			continue;
		}
		let value = if value.contains("://") {
			value.clone()
		} else if names_a_context_source(value) {
			warnings.push(format!(
				"{name}: build.additional_contexts {context_name:?} names {value:?}, which is \
				 neither a path nor a URL; it is not exported"
			));
			continue;
		} else {
			abs_against(base_dir, value)
		};
		add_flag(
			section,
			"--build-context",
			&format!("{context_name}={value}"),
		);
	}
}

/// Whether an `additional_contexts` value names a source instead of pointing
/// at one.
///
/// Compose's `service:other` form means "the image the service `other`
/// builds", which is neither a directory nor a URL. A value shaped like
/// `scheme:rest`, with a scheme of ASCII letters, digits, `+`, `-` or `.`, is
/// such a name. An absolute path is never one, on any platform.
fn names_a_context_source(value: &str) -> bool {
	if Path::new(value).is_absolute() {
		return false;
	}
	match value.split_once(':') {
		Some((scheme, _)) => {
			!scheme.is_empty()
				&& scheme
					.chars()
					.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
		}
		None => false,
	}
}

/// Emit one `--ulimit` flag per `build.ulimits` entry.
///
/// The value is the `name=soft:hard` form `podman build --ulimit` takes. A
/// soft limit above the hard one is clamped, the way the live build path
/// clamps it, because Podman rejects it. A name outside the set of Linux
/// rlimit resources is dropped and reported, the way the live build path
/// drops it: the value reaches a Podman flag, so an unknown name is either
/// a typo or an injection attempt.
fn emit_ulimits(
	name: &str,
	ulimits: &IndexMap<String, UlimitConfig>,
	section: &mut Section,
	warnings: &mut Vec<String>,
) {
	for (limit_name, cfg) in ulimits {
		if !crate::engine::is_known_ulimit(limit_name) {
			warnings.push(format!(
				"{name}: build.ulimits {limit_name:?} is not a recognized resource name and is ignored"
			));
			continue;
		}
		let (soft, hard) = (cfg.soft(), cfg.hard());
		let soft = soft.min(hard);
		add_flag(section, "--ulimit", &format!("{limit_name}={soft}:{hard}"));
	}
}

/// The isolation types `podman build` accepts.
///
/// Buildah parses the value of `--isolation` against this set and refuses
/// anything else with `unrecognized isolation type`, so a value outside it
/// would make the build fail instead of running.
const BUILD_ISOLATION_TYPES: [&str; 3] = ["oci", "rootless", "chroot"];

/// Emit the `--isolation` flag for `build.isolation`.
///
/// Compose's `isolation` names a container isolation technology (`default`,
/// `process`, `hyperv`), not a buildah one, so passing most compose values
/// straight through would make `podman build` refuse to run. Only the buildah
/// types are exported; anything else is reported, which leaves the build with
/// the isolation it had before the field was carried at all.
fn emit_isolation(
	name: &str,
	isolation: Option<&str>,
	section: &mut Section,
	warnings: &mut Vec<String>,
) {
	let Some(value) = isolation else {
		return;
	};
	if value.is_empty() {
		return;
	}
	if !BUILD_ISOLATION_TYPES
		.iter()
		.any(|isolation_type| isolation_type.eq_ignore_ascii_case(value))
	{
		warnings.push(format!(
			"{name}: build.isolation {value:?} is not a `podman build` isolation type \
			 (oci, rootless, chroot) and is not exported"
		));
		return;
	}
	add_flag(section, "--isolation", value);
}
