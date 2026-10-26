//! Which autostart mode, if any, is installed for a project, and the lookup
//! that finds out. The three modes cannot coexist for one project, so at most
//! one is present.

use super::super::paths::unit_path;
use crate::autostart::conflict;

/// Which autostart mode, if any, is installed for a project. The modes cannot
/// coexist (each install refuses the others), so at most one is present. Start
/// mode writes the same unit file as service mode and uninstalls the same way, so
/// it reports as `Service`.
/// `uninstall` uses this to remove whichever is there without the caller naming a
/// mode (and mistakenly no-op'ing against the wrong one).
pub enum InstalledMode {
	/// The service-mode `podup-<project>.service` unit is present.
	Service,
	/// Quadlet `<project>-*.container` units are present.
	Quadlet,
	/// Neither: nothing is installed.
	None,
}

/// Detect the installed autostart mode for `project` from what is on disk.
pub fn installed_mode(project: &str) -> InstalledMode {
	if unit_path(project).exists() {
		InstalledMode::Service
	} else if !conflict::quadlet_units_present(project).is_empty() {
		InstalledMode::Quadlet
	} else {
		InstalledMode::None
	}
}
