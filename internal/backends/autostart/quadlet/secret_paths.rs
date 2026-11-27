//! The path a secret is mounted at, and the one an installed unit already uses.
//!
//! A container unit writes one `Secret=` line per secret a service mounts, and
//! the line carries where the secret lands inside the container: the
//! `/run/secrets/` directory plus the `target=`. A line without a `target=`
//! mounts the secret under its own name, which is what the units of 6.1.0
//! wrote; the units of today name the target after the compose secret, so the
//! file lands at `/run/secrets/<compose name>` instead.
//!
//! Rewriting the units of an install that 6.1.0 made would move every secret
//! to another path without saying so: the container keeps running until it is
//! restarted, and then it cannot find the file it was reading. So an install
//! reads the units it is about to overwrite and, for a secret whose path would
//! move, writes the target the installed unit had, which keeps the file where
//! the service already expects it.
//!
//! The destination the installed unit mounts a secret at is the target it
//! names, or the name of the secret itself when it names none: that is where
//! the file is, whatever the compose file says today. A unit a podup already
//! wrote keeps the path it gave the secret, so a second install does not move
//! a secret the first one left where it was.
//!
//! A compose file that names the target itself keeps its own target: that move
//! is the user's, not ours.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::compose::types::{ComposeFile, Service};
use crate::quadlet::QuadletUnit;

/// Keep the path every secret already has in the units this install overwrites,
/// and report each secret whose path was kept.
///
/// The units are the ones the install is about to write, `installed` the ones
/// of this project it is about to overwrite (already proven to be ours, and to
/// be regular files, by the enumeration that found them), and `file` the
/// compose file the units come from.
///
/// A secret keeps its path when the installed unit mounts it somewhere else
/// than the new unit would, and the compose file does not name the target
/// itself. The destination kept is the one the installed unit mounts it at:
/// the target it names, or the name of the secret itself when it names none.
/// The other options of the line (mode, uid, gid) are the new ones: only the
/// path is what the installed unit decides.
pub(super) fn keep_installed_paths(
	units: &mut [QuadletUnit],
	file: &ComposeFile,
	installed: &[PathBuf],
) -> Vec<String> {
	let mut kept = Vec::new();
	for unit in units.iter_mut() {
		if !unit.filename.ends_with(".container") {
			continue;
		}
		let Some(path) = installed
			.iter()
			.find(|path| path.file_name().and_then(|n| n.to_str()) == Some(unit.filename.as_str()))
		else {
			continue;
		};
		// The read is the capped one the install already uses for the file it
		// is about to overwrite, so a link planted under the name is refused
		// rather than followed, and a unit too large to read is skipped.
		let old = match crate::filesystem::read_to_string_capped(path) {
			Ok(text) => text,
			Err(e) => {
				tracing::warn!(
					"cannot read the installed quadlet unit {}: {e}; the paths of its \
					 secrets are not kept",
					path.display()
				);
				continue;
			}
		};
		let Some(service) = service_of(&unit.contents) else {
			continue;
		};
		let Some(compose) = file.services.get(service.as_str()) else {
			continue;
		};
		let (contents, notes) = keep_unit_paths(&unit.contents, &old, compose);
		if let Some(contents) = contents {
			unit.contents = contents;
		}
		kept.extend(notes);
	}
	kept
}

/// The unit text with the paths of its secrets kept, and the secrets that were
/// kept.
///
/// The compose service is the one the unit is for: the unit writes one
/// `Secret=` line per secret that service mounts, in the order the compose
/// file lists them, so the two pair up by position. A unit that does not
/// match its service is not one this install wrote, and is left alone rather
/// than paired with the wrong secret.
///
/// The destination a secret keeps is the one the installed unit mounts it at:
/// the target that line names, or the name of the secret itself when it names
/// none. The line of the new unit keeps its own name and its own options
/// (mode, uid, gid); only its target is what the installed unit decides.
fn keep_unit_paths(contents: &str, old: &str, compose: &Service) -> (Option<String>, Vec<String>) {
	let lines = secret_lines(contents);
	if lines.len() != compose.secrets.len() {
		return (None, Vec::new());
	}
	let old_entries: Vec<Entry<'_>> = secret_lines(old)
		.iter()
		.map(|entry| Entry::new(entry.1))
		.collect();
	let mut notes = Vec::new();
	let mut changes: HashMap<usize, String> = HashMap::new();
	for (index, entry) in lines.iter().enumerate() {
		let (line, raw) = *entry;
		let parsed = Entry::new(raw);
		let Some(new_target) = parsed.target.as_deref() else {
			continue;
		};
		// The compose file names the target: the move is the user's.
		if compose.secrets[index].target().is_some() {
			continue;
		}
		// The line of the installed unit that mounts this same secret.
		let Some(installed) = old_entries.iter().find(|old| old.name == parsed.name) else {
			continue;
		};
		// The destination the installed unit mounts it at: the target it names,
		// or the name of the secret itself when it names none. That is where
		// the file is, whatever the compose file says today.
		let installed_target = installed
			.target
			.as_deref()
			.unwrap_or(installed.name.as_str());
		if installed_target == new_target {
			continue;
		}
		// The target is written as the installed unit wrote it: the target it
		// names, or its own name when it names none, so the path stays exactly
		// where the service already reads it.
		let target_raw = installed.target_raw.unwrap_or(installed.name_raw);
		changes.insert(line, with_target(raw, target_raw));
		if installed.target.is_none() {
			// The installed unit mounts the secret under its own name: that is
			// the path the units of 6.1.0 gave it, and the user is told about
			// the one the compose file would otherwise move it to.
			let name = &parsed.name;
			notes.push(format!(
				"keeping secret {name} at /run/secrets/{name}, as the installed unit mounts it; \
				 set target: {new_target} in the compose file to move it"
			));
		}
	}
	if changes.is_empty() {
		return (None, notes);
	}
	(Some(replace_lines(contents, &changes)), notes)
}

/// One `Secret=` entry of a container unit.
struct Entry<'a> {
	/// The name field, as it is written.
	name_raw: &'a str,
	/// The name, with the systemd escapes undone.
	name: String,
	/// The target, when the entry names one.
	target: Option<String>,
	/// The target, as it is written.
	target_raw: Option<&'a str>,
}

impl<'a> Entry<'a> {
	fn new(raw: &'a str) -> Self {
		let mut fields = split_quotes(raw).1.split(',');
		let name_raw = fields.next().unwrap_or_default();
		let target_raw = fields.find_map(|field| field.strip_prefix("target="));
		Entry {
			name_raw,
			name: decode(name_raw),
			target: target_raw.map(decode),
			target_raw,
		}
	}
}

/// The lines of a unit's `[Container]` section, with their index in the file.
fn container_lines(contents: &str) -> Vec<(usize, &str)> {
	let mut in_container = false;
	let mut lines = Vec::new();
	for (index, line) in contents.lines().enumerate() {
		let line = line.trim();
		if let Some(section) = line.strip_prefix('[') {
			in_container = section
				.strip_suffix(']')
				.is_some_and(|name| name.trim() == "Container");
			continue;
		}
		if in_container {
			lines.push((index, line));
		}
	}
	lines
}

/// The `Secret=` lines of a unit's `[Container]` section, with their index in
/// the file and the value they carry.
fn secret_lines(contents: &str) -> Vec<(usize, &str)> {
	container_lines(contents)
		.into_iter()
		.filter_map(|(index, line)| line.strip_prefix("Secret=").map(|value| (index, value)))
		.collect()
}

/// The compose service a container unit is for.
///
/// The unit carries the `podup.service` label that names it. The compose
/// file's own `labels:` are written before podup's, so the last one is
/// podup's own.
fn service_of(contents: &str) -> Option<String> {
	let mut service = None;
	for (_, line) in container_lines(contents) {
		if let Some(value) = line.strip_prefix("Label=podup.service=") {
			service = Some(text(value));
		}
	}
	service
}

/// The text of a raw unit value: the quotes and the systemd escapes undone.
fn text(raw: &str) -> String {
	decode(split_quotes(raw).1)
}

/// The body of a raw unit value, and whether it is wrapped in quotes.
fn split_quotes(raw: &str) -> (bool, &str) {
	let raw = raw.trim_end();
	match raw.strip_prefix('"') {
		Some(rest) => match rest.strip_suffix('"') {
			Some(body) => (true, body),
			None => (false, raw),
		},
		None => (false, raw),
	}
}

/// The text of a raw option field: the systemd escapes undone.
///
/// A quoted value keeps the escape sequences the writer wrote, so `\\` is one
/// backslash and `\"` one quote. The `%%` and `$$` pairs are the specifier and
/// the variable escapes systemd reads as one `%` and one `$`.
fn decode(raw: &str) -> String {
	let mut out = String::with_capacity(raw.len());
	let mut chars = raw.chars();
	while let Some(c) = chars.next() {
		if c != '\\' {
			out.push(c);
			continue;
		}
		match chars.next() {
			Some('n') => out.push('\n'),
			Some('r') => out.push('\r'),
			Some('t') => out.push('\t'),
			Some('\\') => out.push('\\'),
			Some('"') => out.push('"'),
			Some(other) => {
				out.push('\\');
				out.push(other);
			}
			None => out.push('\\'),
		}
	}
	out.replace("%%", "%").replace("$$", "$")
}

/// The `Secret=` value with its `target=` option set to `target`.
///
/// The target is written as the unit it comes from wrote it, so the value keeps
/// the escapes that unit already uses for it.
fn with_target(raw: &str, target: &str) -> String {
	let (quoted, body) = split_quotes(raw);
	let mut out = String::with_capacity(raw.len());
	for (index, field) in body.split(',').enumerate() {
		if index > 0 {
			out.push(',');
		}
		if field.starts_with("target=") {
			out.push_str("target=");
			out.push_str(target);
		} else {
			out.push_str(field);
		}
	}
	if quoted {
		format!("\"{out}\"")
	} else {
		out
	}
}

/// The unit text with the given lines replaced, by their index in the file.
fn replace_lines(contents: &str, changes: &HashMap<usize, String>) -> String {
	let mut out = String::with_capacity(contents.len());
	for (index, chunk) in contents.split_inclusive('\n').enumerate() {
		match changes.get(&index) {
			None => out.push_str(chunk),
			Some(value) => {
				out.push_str("Secret=");
				out.push_str(value);
				if chunk.ends_with('\n') {
					out.push('\n');
				}
			}
		}
	}
	out
}
