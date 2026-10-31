//! Render compose values into Quadlet directive strings, and the `[Section]`
//! accumulator plus the sanitizing helpers shared across the generator.

use std::collections::BTreeMap;

use crate::compose::types::RestartPolicy;
#[cfg(test)]
use crate::compose::types::{Command, VolumeMount, VolumeType};
use crate::ports::ParsedPort;

mod exec;
mod mounts;
mod unit;

pub(crate) use exec::has_lone_semicolon_word;
pub(super) use exec::render_command;
pub(super) use mounts::{render_tmpfs_mount, render_volume};
pub(super) use unit::Section;

#[cfg(test)]
use unit::escape_unit_value;

#[cfg(test)]
use exec::exec_words;

/// Render a parsed port into a Quadlet `PublishPort=` value
/// (`[host_ip:][host_port:]container[/proto]`). A missing or `0` host port is
/// omitted so Podman picks one; the protocol suffix is only added when it is not
/// the default `tcp`.
pub(super) fn render_publish_port(p: &ParsedPort) -> String {
	let mut s = String::new();
	if !p.host_ip.is_empty() {
		s.push_str(&bracket_ipv6(&p.host_ip));
		s.push(':');
	}
	// A host port of None/0 means "let Podman pick"; omit it so the published
	// side is empty (PublishPort=<container>) and Podman assigns a port.
	if let Some(host) = p.host_port.filter(|n| *n != 0) {
		s.push_str(&host.to_string());
		s.push(':');
	}
	s.push_str(&p.container_port.to_string());
	if p.protocol != "tcp" {
		s.push('/');
		s.push_str(&p.protocol);
	}
	s
}

/// Wrap an IPv6 host address in the brackets Podman expects in a
/// `PublishPort=` value.
///
/// The address carries colons, so without the brackets Podman reads the
/// first of them as the separator between the address and the port. An
/// address that already carries the brackets is left alone.
fn bracket_ipv6(host_ip: &str) -> String {
	if host_ip.contains(':') && !(host_ip.starts_with('[') && host_ip.ends_with(']')) {
		format!("[{host_ip}]")
	} else {
		host_ip.to_string()
	}
}

/// The names the external networks and volumes really have on the host, keyed
/// by the compose key that refers to them.
///
/// A compose `name:` overrides the key, so the unit has to name the resource
/// by that name: referring to it by the compose key would point the container
/// at a resource that does not exist, or at one that is not the external one.
pub(super) type ExternalNames<'a> = std::collections::HashMap<&'a str, &'a str>;

/// Map a compose `restart:` policy onto a systemd `Restart=` value.
/// `unless-stopped` has no systemd equivalent and is treated as `always`.
pub(super) fn render_restart(restart: &RestartPolicy) -> String {
	match restart {
		RestartPolicy::No => "no".to_string(),
		RestartPolicy::Always => "always".to_string(),
		RestartPolicy::UnlessStopped => "always".to_string(),
		RestartPolicy::OnFailure { .. } => "on-failure".to_string(),
	}
}

/// Sort a map of optional-valued entries by key into a stable `Vec`, so the
/// generated unit is byte-deterministic regardless of `HashMap` iteration order.
pub(super) fn sorted_pairs(
	map: std::collections::HashMap<String, Option<String>>,
) -> Vec<(String, Option<String>)> {
	let sorted: BTreeMap<_, _> = map.into_iter().collect();
	sorted.into_iter().collect()
}

/// Sort a map of string-valued entries (labels, sysctls, …) by key into a stable
/// `Vec`, so the generated unit is byte-deterministic regardless of `HashMap`
/// iteration order.
pub(super) fn sorted_label_pairs(
	map: std::collections::HashMap<String, String>,
) -> Vec<(String, String)> {
	let sorted: BTreeMap<_, _> = map.into_iter().collect();
	sorted.into_iter().collect()
}

/// Strip ASCII control characters from a value before it is written into a
/// unit file. systemd unit entries are single-line `Key=Value` pairs; an
/// embedded newline from a hostile compose field would otherwise inject
/// arbitrary unit directives (e.g. a `[Service]` `ExecStartPre=`). Compose
/// input is untrusted, so every dynamic value is sanitized at the boundary.
pub(super) fn sanitize_value(value: &str) -> String {
	value.chars().filter(|c| !c.is_control()).collect()
}

/// Quote a compose value that is interpolated into a `PodmanArgs=` template
/// (one of the seven [`crate::quadlet::render`] sites), so the resulting
/// `PodmanArgs=` line cannot smuggle additional `podman run` flags into the
/// argv the quadlet generator hands to `podman`.
///
/// Three things happen, in order:
///
/// 1. Control characters are stripped, the same rule as [`sanitize_value`], so
///    an embedded newline cannot split the directive or inject a sibling
///    line.
/// 2. `%` is doubled, because systemd specifiers like `%h` or `%U` would otherwise
///    be expanded at unit activation, and a value like `%h/mem` would become
///    `<hostname>/mem` inside the podman flag. `Environment=` already does
///    this; the seven `PodmanArgs=` interpolation sites were not.
/// 3. The result is wrapped in double quotes (with `"` and `\` escaped),
///    which is the systemd word-splitting syntax for "these characters are
///    one token". A hostile value carrying whitespace stays inside the
///    quoted group; podman sees one argv element and rejects it.
pub(super) fn quote_podman_arg_value(value: &str) -> String {
	let stripped = sanitize_value(value);
	let escaped = stripped.replace('%', "%%");
	format!("\"{}\"", escaped.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Build the project-prefixed unit-file stem for a resource, e.g. service `web`
/// in project `proj` → `proj-web`. Generated unit files share a single systemd
/// directory across projects, so the stem (and every in-unit cross-reference to
/// it) carries the project name, matching the project-scoped resource names
/// inside the units, so two projects' `web` services do not clobber each other.
pub(super) fn unit_stem(project: &str, name: &str) -> String {
	safe_unit_stem(&format!("{project}-{name}"))
}

/// Reduce a compose key to a safe single path-component stem for a unit file
/// name. Keeps ASCII alphanumerics and `-`/`_`/`.`; replaces anything else
/// (path separators, control characters) with `_`, and guarantees the result
/// is non-empty and starts with neither a dot nor a dash, so it can never escape
/// the output directory, resolve to `.`/`..`, or be mistaken for a command-line
/// flag by downstream tooling that globs the generated file names.
pub(super) fn safe_unit_stem(name: &str) -> String {
	let mut stem: String = name
		.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
				c
			} else {
				'_'
			}
		})
		.collect();
	if stem.is_empty() || stem.starts_with('.') || stem.starts_with('-') {
		stem.insert(0, '_');
	}
	stem
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod tests;
