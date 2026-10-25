//! `podup autostart` service mode: a single rootless `systemctl --user` unit that
//! brings a compose stack up at boot.
//!
//! Everything here is user-scope only: the unit lives under
//! `${XDG_CONFIG_HOME:-~/.config}/systemd/user/` and every action goes through
//! `systemctl --user` / `loginctl`. No root, no `sudo`, nothing under `/etc` or
//! the system systemd. External-command calls go through the `SystemCtl` seam so
//! the install/uninstall/status logic is unit-testable without a live systemd.

mod conflict;
mod detect;
mod guards;
mod install;
mod options;
mod paths;
mod quadlet;
mod service;
mod start;
mod status;
mod systemctl;

#[cfg(test)]
#[path = "start_tests.rs"]
mod start_tests;

#[cfg(test)]
#[path = "unit_quoting_tests.rs"]
mod unit_quoting_tests;

// Unix only: it reuses the fixtures in `tests.rs`, which is Unix-only.
#[cfg(all(test, unix))]
#[path = "conflict_tests.rs"]
mod conflict_tests;

pub use detect::{installed_mode, InstalledMode};
pub use install::{install, install_start, uninstall};
pub use options::InstallOptions;
pub use paths::max_stop_grace_secs;
pub use quadlet::{install_quadlet, rebuild_quadlet, uninstall_quadlet};
pub use service::{
	render_service_unit, render_update_service_unit, render_update_timer_unit,
	update_service_file_name, update_timer_file_name, validate_auto_update_interval,
	ServiceUnitOpts,
};
pub use start::{
	render_start_unit, sole_container, validate_start_unit_opts, StartModeRefusal, StartUnitOpts,
};
pub use status::{collect_status, status, NetworkWait, StatusReport};
pub use systemctl::{RealSystemCtl, SystemCtl};

#[cfg(all(test, unix))]
mod tests;
