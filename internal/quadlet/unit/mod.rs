//! Build the individual `.network`, `.volume` and `.container` units.

mod build;
mod container;
mod health;
mod network;
mod security;
mod volume;

pub(super) use build::build_unit;
pub(super) use container::container_unit;
pub(super) use network::network_unit;
pub(super) use volume::volume_unit;

// Shared helpers from the sibling `render`/`warnings` modules and the parent
// `QuadletUnit` type, re-exported so the unit submodules import them from here.
use super::render::{
	render_command, render_publish_port, render_restart, render_tmpfs_mount, render_volume,
	sorted_label_pairs, sorted_pairs, unit_stem, Section,
};
use super::warnings::collect_warnings;
use super::QuadletUnit;

/// The ownership marker every generated unit carries as its literal first
/// line: a `#` comment, ignored by systemd, naming the project that owns the
/// unit.
///
/// This is deliberately separate from the `Label=podup.project=<project>`
/// line each unit also carries (kept for runtime scoping — Podman uses it for
/// container/secret lookups). A compose service's user-supplied `labels:` are
/// rendered into the same section as that `Label=` line, in the same
/// `Key=Value` shape, so a service declaring `labels: {podup.project: other}`
/// produces an indistinguishable forged `Label=podup.project=other` line
/// ahead of the real one. A `#`-prefixed line cannot be forged the same way:
/// compose labels only ever become `Label=key=value` entries, never a
/// comment, so this marker is the line ownership checks (`unit_owner` in
/// `crate::autostart::quadlet`) must read instead.
fn owner_marker(project: &str) -> String {
	format!("# podup-owner: {project}\n")
}
