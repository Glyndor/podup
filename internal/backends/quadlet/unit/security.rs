//! Render service `secrets:` and `security_opt:` onto their Quadlet keys.

use indexmap::IndexMap;

use crate::compose::types::{SecretConfig, ServiceSecretRef};

use super::{quote_podman_arg_value, Section};

/// Sanitize one `Secret=` option-list field: drop control characters and the
/// `,`/`=` separators so a hostile compose value cannot inject extra options.
pub(super) fn secret_field(value: &str) -> String {
	value
		.chars()
		.filter(|c| !c.is_control() && *c != ',' && *c != '=')
		.collect()
}

/// Whether a top-level secret definition is one `up` creates as a project-scoped
/// native Podman secret: a `content:`, `environment:` or `file:` source.
/// `external:` wins (never created by podup) and an empty def is left alone.
pub(super) fn is_inline_secret(def: Option<&SecretConfig>) -> bool {
	def.is_some_and(|d| {
		d.external != Some(true)
			&& (d.content.is_some() || d.environment.is_some() || d.file.is_some())
	})
}

/// Resolve the `Secret=` source name for a service secret reference. A secret
/// `up` creates resolves to the project-scoped name `{project}_secret_{name}`
/// (via `plan::scoped_name`), so generated units reference the same secret `up`
/// would; an `external:` one resolves to its `name:` when it has one, as the
/// engine does; any other keeps its compose source name.
fn secret_source_name(
	project: &str,
	source: &str,
	secrets: &IndexMap<String, SecretConfig>,
) -> String {
	let def = secrets.get(source);
	if is_inline_secret(def) {
		format!("{project}_secret_{source}")
	} else if let Some(name) = def
		.filter(|d| d.external == Some(true))
		.and_then(|d| d.name.as_deref())
	{
		name.to_string()
	} else {
		source.to_string()
	}
}

/// Render a service `secrets:` entry into a Quadlet `Secret=` value
/// (`name[,target=,uid=,gid=,mode=]`), resolving inline secrets to their
/// project-scoped name so the reference matches what `up` creates.
pub(super) fn render_secret(
	secret: &ServiceSecretRef,
	project: &str,
	secrets: &IndexMap<String, SecretConfig>,
) -> String {
	match secret {
		// Sanitize the short-form name too: `Secret=` is an option list, so a
		// `,`/`=` in the name would inject extra options (same guard as Long).
		ServiceSecretRef::Short(name) => secret_field(&secret_source_name(project, name, secrets)),
		ServiceSecretRef::Long {
			source,
			target,
			uid,
			gid,
			mode,
		} => {
			// `Secret=` is a comma-separated `key=value` option list, so a `,`
			// or `=` embedded in any field would inject extra options. Strip
			// those (and control chars) from each value at the boundary.
			let mut s = secret_field(&secret_source_name(project, source, secrets));
			if let Some(t) = target {
				s.push_str(&format!(",target={}", secret_field(t)));
			}
			if let Some(u) = uid {
				s.push_str(&format!(",uid={}", secret_field(u)));
			}
			if let Some(g) = gid {
				s.push_str(&format!(",gid={}", secret_field(g)));
			}
			if let Some(m) = mode {
				s.push_str(&format!(",mode={m:o}"));
			}
			s
		}
	}
}

/// Map a single compose `security_opt` entry onto the dedicated Quadlet key
/// where one exists; unrecognized entries are reported rather than dropped.
pub(super) fn map_security_opt(
	opt: &str,
	container: &mut Section,
	name: &str,
	warnings: &mut Vec<String>,
) {
	// Compose writes `label:disable` and `seccomp:unconfined`, with a colon, and
	// Podman reads both separators (the engine does too). The branches below are
	// written for the `=` form, so the first separator of these two is normalised.
	let canonical;
	let opt = if let Some(rest) = opt.strip_prefix("seccomp:") {
		canonical = format!("seccomp={rest}");
		canonical.as_str()
	} else if let Some(rest) = opt.strip_prefix("label:") {
		canonical = format!("label={rest}");
		canonical.as_str()
	} else {
		opt
	};
	if let Some(rest) = opt.strip_prefix("no-new-privileges") {
		// The value starts after the first separator, the way the engine reads
		// the same entry, so both paths read one compose file the same way.
		let val = rest.strip_prefix([':', '=']).unwrap_or(rest);
		if val.is_empty() {
			container.add("NoNewPrivileges", "true".to_string());
		} else if let Some(enabled) = crate::engine::container::parse_bool_value(val) {
			container.add("NoNewPrivileges", enabled.to_string());
		} else {
			// A value Podman does not read as a boolean is not one it would
			// accept either. Turning it into `false` would disable the
			// hardening without saying so, so the entry is reported and
			// skipped instead.
			warnings.push(format!(
				"{name}: security_opt '{opt}' carries a value Podman does not read as a boolean and is skipped"
			));
		}
	} else if let Some(profile) = opt.strip_prefix("seccomp=") {
		container.add("SeccompProfile", profile.to_string());
	} else if let Some(profile) = opt
		.strip_prefix("apparmor=")
		.or_else(|| opt.strip_prefix("apparmor:"))
	{
		// `AppArmor=` is not a recognised [Container] Quadlet key (Quadlet would
		// drop the whole unit at daemon-reload), so route it through PodmanArgs= as
		// `--security-opt apparmor=<profile>`, like the other escape-hatch flags.
		// `profile` is composed input; a value like `my-profile --privileged`
		// must stay one argv element rather than two podman args: `escape_unit_value`
		// short-circuits on `PodmanArgs` (#1734), so the quote happens here.
		container.add(
			"PodmanArgs",
			format!(
				"--security-opt apparmor={}",
				quote_podman_arg_value(profile)
			),
		);
	} else if let Some(label) = opt.strip_prefix("label=") {
		if label == "disable" {
			container.add("SecurityLabelDisable", "true".to_string());
		} else if label == "nested" {
			container.add("SecurityLabelNested", "true".to_string());
		} else if let Some(t) = label.strip_prefix("type:") {
			container.add("SecurityLabelType", t.to_string());
		} else if let Some(l) = label.strip_prefix("level:") {
			container.add("SecurityLabelLevel", l.to_string());
		} else if let Some(f) = label.strip_prefix("filetype:") {
			container.add("SecurityLabelFileType", f.to_string());
		} else {
			warnings.push(format!(
				"{name}: security_opt 'label={label}' has no Quadlet key and is skipped"
			));
		}
	} else if let Some(paths) = opt.strip_prefix("mask=") {
		container.add("Mask", paths.to_string());
	} else if let Some(paths) = opt.strip_prefix("unmask=") {
		container.add("Unmask", paths.to_string());
	} else {
		warnings.push(format!(
			"{name}: security_opt '{opt}' has no Quadlet mapping and is skipped"
		));
	}
}

#[cfg(test)]
#[path = "security_tests.rs"]
mod tests;
