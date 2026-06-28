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
