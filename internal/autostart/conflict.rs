//! Refusals that keep two autostart modes from being installed for one project.
//!
//! All three modes bring the same stack up at boot. Service and start mode also
//! write the same file, `podup-<project>.service`, so without a check the second
//! install silently replaces the first, and a service-mode `--auto-update` timer
//! is left firing against a start-mode unit (#1954). Every check here runs
//! before anything is written, so `--dry-run` reports the same outcome.

use std::path::PathBuf;

use crate::ComposeError;

use super::{config_home, unit_dir, unit_file_name};

/// Quadlet autostart units for this project, if any exist on disk. Service mode
/// and Quadlet mode would both try to start the same stack at boot, so an
/// existing Quadlet install is a conflict to surface, not to silently overwrite.
/// Looks for `<project>-*.container` under
/// `${XDG_CONFIG_HOME:-~/.config}/containers/systemd/`.
pub(super) fn quadlet_units_present(project: &str) -> Vec<PathBuf> {
	let dir = config_home().join("containers").join("systemd");
	let prefix = format!("{project}-");
	let mut found = Vec::new();
	if let Ok(entries) = std::fs::read_dir(&dir) {
		for entry in entries.flatten() {
			let name = entry.file_name();
			let name = name.to_string_lossy();
			if name.starts_with(&prefix) && name.ends_with(".container") {
				found.push(entry.path());
			}
		}
	}
	found.sort();
	found
}

/// Refuse to stack a single-unit mode on top of an existing Quadlet autostart
/// install for the same project: both would start the stack at boot.
pub(super) fn refuse_if_quadlet_present(project: &str) -> crate::Result<()> {
	let quadlet = quadlet_units_present(project);
	if quadlet.is_empty() {
		return Ok(());
	}
	let names: Vec<String> = quadlet.iter().map(|p| p.display().to_string()).collect();
	Err(ComposeError::Autostart(format!(
		"quadlet autostart units for project '{project}' already exist:\n    {}\n\
		 remove them before installing another mode (quadlet autostart is tracked by #993).",
		names.join("\n    ")
	)))
}

/// What the `podup-<project>.service` file on disk (when present) was written
/// for. `Unknown` means the file exists but its `ExecStart=` line matches
/// neither shape, and overwriting it would be silent corruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DetectedSingleUnitMode {
	/// Service mode: `ExecStart=... up -d ...` (with or without
	/// `--no-build --pull never`, the latter being the current shape and the
	/// former what older releases wrote).
	Service,
	/// Start mode: `ExecStart=<podman> start <container>`.
	Start,
	/// The file is there, but its `ExecStart=` is not a known shape.
	Unknown,
}

/// Which single-unit mode an installer is about to write. The refusal compares
/// this against what is on disk for the same project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SingleUnitMode {
	Service,
	Start,
}

impl SingleUnitMode {
	fn name(self) -> &'static str {
		match self {
			Self::Service => "service",
			Self::Start => "start",
		}
	}
}

impl DetectedSingleUnitMode {
	fn name(self) -> &'static str {
		match self {
			Self::Service => "service",
			Self::Start => "start",
			Self::Unknown => "unknown",
		}
	}
}

/// Split one systemd `ExecStart=` line into the arguments it represents,
/// honouring the double-quoted form `quote_arg` produces. A backslash inside a
/// quoted run escapes the next character (the C-escapes `quote_arg` actually
/// emits: `\"`, `\\`, `\n`, `\t`, `\r`); everything else is literal until the
/// closing `"`. Unquoted whitespace is a separator, the rest of the line is one
/// token. Empty input yields no tokens, not a single empty one.
fn tokenize_exec(line: &str) -> Vec<String> {
	let mut tokens = Vec::new();
	let mut current = String::new();
	let mut in_quotes = false;
	let mut chars = line.chars().peekable();
	while let Some(c) = chars.next() {
		if in_quotes {
			if c == '\\' {
				if let Some(next) = chars.next() {
					current.push(next);
				} else {
					current.push('\\');
				}
			} else if c == '"' {
				in_quotes = false;
			} else {
				current.push(c);
			}
		} else if c == '"' {
			in_quotes = true;
		} else if c.is_whitespace() {
			if !current.is_empty() {
				tokens.push(std::mem::take(&mut current));
			}
		} else {
			current.push(c);
		}
	}
	if !current.is_empty() {
		tokens.push(current);
	}
	tokens
}

/// Classify the mode that produced a `podup-<project>.service` from its
/// `ExecStart=` line. Strict on tokens, not on substrings, so a project or
/// container whose name happens to be `start` or `up` cannot be misread.
pub(super) fn classify_exec_start(line: &str) -> DetectedSingleUnitMode {
	let tokens = tokenize_exec(line);
	let n = tokens.len();
	// Start mode renders exactly `<podman> start <container>`: three tokens
	// with `start` in the middle. A path or container named `start` cannot
	// satisfy this shape (it would have to be the second token, and the line
	// would have to be three tokens long).
	if n == 3 && tokens[1] == "start" {
		return DetectedSingleUnitMode::Start;
	}
	// Service mode renders `<podup>`, then `-f`, `-p`, `--profile` and
	// `--env-file` each with a value, then `up -d` and whatever flags that
	// release put after it (plain `up -d`, `up -d --build` in 1.9.x, and
	// `up -d --no-build --pull never` now). Skipping the option pairs means a
	// project or file literally named `up` is read as a value, not as the verb.
	let mut i = 1;
	while i + 1 < n && matches!(tokens[i].as_str(), "-f" | "-p" | "--profile" | "--env-file") {
		i += 2;
	}
	if i + 1 < n && tokens[i] == "up" && tokens[i + 1] == "-d" {
		return DetectedSingleUnitMode::Service;
	}
	DetectedSingleUnitMode::Unknown
}

/// Detect the installed single-unit mode for `project` from the unit file on
/// disk. `None` only when no file exists: one that cannot be read, or has no
/// `ExecStart=`, is `Unknown`, so it is refused rather than overwritten.
pub(super) fn installed_single_unit_mode(project: &str) -> Option<DetectedSingleUnitMode> {
	let path = unit_dir().join(unit_file_name(project));
	if !path.exists() {
		return None;
	}
	let mode = std::fs::read_to_string(&path)
		.ok()
		.and_then(|body| {
			body.lines()
				.find_map(|l| l.strip_prefix("ExecStart="))
				.map(classify_exec_start)
		})
		.unwrap_or(DetectedSingleUnitMode::Unknown);
	Some(mode)
}

/// Refuse to install `incoming` when a different single-unit mode is already
/// installed for the same project. Re-installing the SAME mode is allowed:
/// that is the documented upgrade path, and `place_unit` rewrites the file in
/// place either way.
pub(super) fn refuse_if_other_single_unit_mode(
	project: &str,
	incoming: SingleUnitMode,
) -> crate::Result<()> {
	let Some(detected) = installed_single_unit_mode(project) else {
		return Ok(());
	};
	let incoming_name = incoming.name();
	match (detected, incoming) {
		(DetectedSingleUnitMode::Service, SingleUnitMode::Service)
		| (DetectedSingleUnitMode::Start, SingleUnitMode::Start) => Ok(()),
		(DetectedSingleUnitMode::Service, SingleUnitMode::Start)
		| (DetectedSingleUnitMode::Start, SingleUnitMode::Service) => {
			Err(ComposeError::Autostart(format!(
				"a {detected} autostart unit for '{project}' is already installed at {path}; \
				 remove it with `podup autostart uninstall` before installing {incoming_name} mode \
				 (both would start the same stack at boot and the second would overwrite the first \
				 silently).",
				detected = detected.name(),
				incoming_name = incoming_name,
				path = unit_dir().join(unit_file_name(project)).display(),
			)))
		}
		(DetectedSingleUnitMode::Unknown, _) => Err(ComposeError::Autostart(format!(
			"a `podup-{project}.service` unit is already installed at {path} but its ExecStart= \
			 line does not match either service or start mode; refusing to overwrite a unit \
			 podup cannot identify. Remove it with `podup autostart uninstall` before installing \
			 {incoming_name} mode.",
			path = unit_dir().join(unit_file_name(project)).display(),
		))),
	}
}
