//! Who owns a unit file that podup may have written.
//!
//! Two shapes can sit at a unit's path: one podup wrote, carrying the
//! `# podup-owner: <project>` marker it stamps as the first line, and one
//! written before the marker existed, which is recognised by the
//! `Description=` line podup has always rendered. Both answers are needed
//! before a unit is written or removed, because the unit names are derived
//! from a project name and a project name can be a prefix of another
//! project's (`demo` and `demo-update`).

use std::path::Path;

/// The project a unit file belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) enum Owner {
	/// The file carries the `# podup-owner: <project>` marker podup stamps.
	Marked(String),
	/// The file predates the marker: its `Description=` names the project and
	/// the role it was written for.
	Legacy {
		/// The project the unit belongs to.
		project: String,
		/// What the unit does.
		role: Role,
	},
}

/// What a legacy (pre-marker) unit does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) enum Role {
	/// The project's main unit, service mode or start mode.
	Main,
	/// The auto-update oneshot.
	UpdateService,
	/// The auto-update timer.
	UpdateTimer,
}

impl Owner {
	/// The project this unit belongs to.
	pub(in super::super) fn project(&self) -> &str {
		match self {
			Owner::Marked(project) => project,
			Owner::Legacy { project, .. } => project,
		}
	}
}

/// The marker line podup stamps as the first line of every unit it writes.
///
/// A comment, so systemd ignores it, and a line a compose file cannot forge:
/// compose `labels:` render as `Key=Value` entries, never as comments.
pub(in super::super) fn marker(project: &str) -> String {
	format!("# podup-owner: {project}")
}

/// The owner of a unit file's contents, when the contents prove one.
pub(in super::super) fn owner_of_text(text: &str) -> Option<Owner> {
	if let Some(project) = text
		.lines()
		.find_map(|line| line.strip_prefix("# podup-owner: "))
	{
		let project = project.trim();
		if !project.is_empty() {
			return Some(Owner::Marked(project.to_string()));
		}
	}
	let description = text
		.lines()
		.find_map(|line| line.strip_prefix("Description="))?;
	let rest = description.strip_prefix("podup ")?.trim();
	// Longest suffix first: the timer's description ends with the oneshot's
	// description plus " timer".
	let (project, role) = if let Some(project) = rest.strip_suffix(" auto-update timer") {
		(project, Role::UpdateTimer)
	} else if let Some(project) = rest.strip_suffix(" auto-update") {
		(project, Role::UpdateService)
	} else {
		(rest, Role::Main)
	};
	if project.is_empty() {
		return None;
	}
	Some(Owner::Legacy {
		project: project.to_string(),
		role,
	})
}

/// The owner of the unit file at `path`, when the file proves one.
///
/// A link is never followed: the file it points at is not the unit at this
/// path, so it says nothing about who owns the name.
pub(in super::super) fn owner_of_file(path: &Path) -> Option<Owner> {
	let meta = std::fs::symlink_metadata(path).ok()?;
	if !meta.file_type().is_file() {
		return None;
	}
	let text = std::fs::read_to_string(path).ok()?;
	owner_of_text(&text)
}

/// The role of a unit written before the marker existed, when the file proves
/// one. A marked file has no role to report: the marker says who owns it, not
/// what it was written for.
pub(in super::super) fn legacy_role(path: &Path) -> Option<Role> {
	match owner_of_file(path) {
		Some(Owner::Legacy { role, .. }) => Some(role),
		_ => None,
	}
}
