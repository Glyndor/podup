//! `podup autostart status`: the network-ordering probe (`#1616`), the linger /
//! session snapshot, the unit file's on-disk state, and the rendering that
//! turns all of that into the `status` table.

use std::path::PathBuf;

use super::guards::{current_user, linger_enabled};
use super::paths::{file_mode, unit_file_name, unit_path};

/// Whether the network-ordering the generated units declare is real.
///
/// `Wants=`/`After=` naming a unit systemd cannot load is dropped silently:
/// `LoadState` reads `not-found`, the dependent starts clean, and nothing
/// reaches the journal. So a unit file can say it waits for the network while
/// waiting for nothing, and the only way to tell is to ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkWait {
	/// systemd loaded the shim, so the ordering the unit declares takes effect.
	Loaded,
	/// No such unit. Podman ships it from 5.3.0; below that the ordering in
	/// every autostart unit is inert.
	NotFound,
	/// `systemctl` could not be asked, or answered something unrecognised.
	/// Carried rather than collapsed into `NotFound`: "we could not tell" and
	/// "it is not there" are different answers and only one is a problem.
	Unknown(String),
}

/// A snapshot of the autostart unit's state, gathered for `status`.
pub struct StatusReport {
	/// Absolute path to where the unit file would live.
	pub unit_path: PathBuf,
	/// Whether the unit file exists on disk.
	pub unit_exists: bool,
	/// The unit file's permission bits (Unix only), when it exists.
	pub unit_mode: Option<u32>,
	/// `systemctl --user is-active` output (e.g. `active`, `inactive`, `failed`).
	pub is_active: String,
	/// `systemctl --user is-enabled` output (e.g. `enabled`, `disabled`).
	pub is_enabled: String,
	/// Whether linger is enabled for the current user.
	pub linger: bool,
	/// Whether `XDG_RUNTIME_DIR` is set (a user session is present).
	pub runtime_dir: bool,
	/// Whether the network shim the generated units order against is loadable.
	pub network_wait: NetworkWait,
}

/// The unit every autostart mode orders against, so the name lives in one place.
pub(crate) const NETWORK_SHIM: &str = "podman-user-wait-network-online.service";

/// Run a `systemctl --user` query that reports state through its stdout (e.g.
/// `is-active`, `is-enabled`); these exit non-zero for the negative answer, so
/// the trimmed stdout is the report regardless of exit status.
fn query<S: super::systemctl::SystemCtl>(sc: &S, arg: &str, unit_name: &str) -> String {
	match sc.systemctl(&[arg, unit_name]) {
		Ok(out) => {
			let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
			if s.is_empty() {
				"unknown".to_string()
			} else {
				s
			}
		}
		Err(e) => format!("unknown ({e})"),
	}
}

/// Ask systemd whether the network shim is loadable.
///
/// **The exit code is not the signal, and must not be used as one.** Measured
/// 2026-08-30 on Podman 5.7.0, rootless: `systemctl --user show <unit> -p
/// LoadState` exits **0 for a unit that does not exist** just as it does for one
/// that does, printing `LoadState=not-found` in the first case and
/// `LoadState=loaded` in the second. A guard branching on the status would
/// report the shim as fine while it is missing, which is the same vacuous check
/// this function exists to replace.
///
/// That differs from the two verbs the rest of this status uses: `is-active`
/// and `is-enabled` both exit **4** for an unknown unit. Two conventions, one
/// module, so the difference is stated here rather than left to be rediscovered.
fn network_wait_state<S: super::systemctl::SystemCtl>(sc: &S) -> NetworkWait {
	let out = match sc.systemctl(&["show", NETWORK_SHIM, "-p", "LoadState"]) {
		Ok(out) => out,
		Err(e) => return NetworkWait::Unknown(format!("systemctl show failed: {e}")),
	};
	let text = String::from_utf8_lossy(&out.stdout);
	let Some(state) = text
		.lines()
		.find_map(|l| l.trim().strip_prefix("LoadState="))
	else {
		return NetworkWait::Unknown("systemctl show printed no LoadState".to_string());
	};
	match state.trim() {
		"loaded" => NetworkWait::Loaded,
		"not-found" => NetworkWait::NotFound,
		other => NetworkWait::Unknown(other.to_string()),
	}
}

/// Gather the autostart status for a project, going through the [`SystemCtl`](crate::autostart::SystemCtl) seam
/// so it is testable without a live systemd.
pub fn collect_status<S: super::systemctl::SystemCtl>(sc: &S, project: &str) -> StatusReport {
	let unit_name = unit_file_name(project);
	let path = unit_path(project);
	let unit_exists = path.exists();
	StatusReport {
		unit_mode: if unit_exists { file_mode(&path) } else { None },
		unit_exists,
		unit_path: path,
		is_active: query(sc, "is-active", &unit_name),
		is_enabled: query(sc, "is-enabled", &unit_name),
		linger: current_user().is_some_and(|u| linger_enabled(sc, &u)),
		runtime_dir: std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|s| !s.is_empty()),
		network_wait: network_wait_state(sc),
	}
}

/// Print the autostart status for a project.
pub fn status<S: super::systemctl::SystemCtl>(sc: &S, project: &str) -> crate::Result<()> {
	let r = collect_status(sc, project);
	// Every line here is a yes/no an operator is scanning for, and the whole
	// screen was one colour, so "is it actually running?" meant reading six
	// labels to find the one word that answers it. The label is scaffolding
	// (dimmed); the value carries the meaning (tinted by what it says).
	let row = |label: &str, value: &str| {
		crate::ui::print_labelled(label, value);
	};
	row("unit", &r.unit_path.display().to_string());
	row("installed", if r.unit_exists { "yes" } else { "no" });
	if let Some(mode) = r.unit_mode {
		row("mode", &format!("{:04o}", mode & 0o7777));
	}
	// With no unit file on disk, systemd's answers to both questions are the same
	// negative the `installed: no` line above already gave, and neither is a
	// failure, but `is-enabled` returns the word `not-found`, which the status
	// vocabulary reads as an error and paints red. So an uninstalled project
	// reported one dim `no` and one red `not-found` about the same unit.
	//
	// A unit that *is* on disk while systemd still cannot find it stays red: that
	// is a genuinely broken install, and the two cases must not look alike.
	if r.unit_exists {
		row("active", &r.is_active);
		row("enabled", &r.is_enabled);
	} else {
		crate::ui::print_labelled_neutral("active", &r.is_active);
		crate::ui::print_labelled_neutral("enabled", &r.is_enabled);
	}
	row("linger", if r.linger { "enabled" } else { "disabled" });
	// Prose for the same reason the session line is prose: "not-found" alone
	// would read as a broken install rather than as the ordering being inert,
	// and the operator needs to be told what it costs and what fixes it.
	match &r.network_wait {
		NetworkWait::Loaded => crate::ui::print_labelled_with(
			"network wait",
			&format!("{NETWORK_SHIM} is loaded"),
			Some(true),
		),
		NetworkWait::NotFound => crate::ui::print_labelled_with(
			"network wait",
			&format!(
				"{NETWORK_SHIM} is not loadable, so the unit's network ordering \
				 is dropped silently (Podman ships it from 5.3.0)"
			),
			Some(false),
		),
		NetworkWait::Unknown(why) => {
			crate::ui::print_labelled_neutral("network wait", &format!("unknown ({why})"))
		}
	}
	// Prose, not a state word, so the meaning is stated rather than inferred.
	crate::ui::print_labelled_with(
		"session",
		if r.runtime_dir {
			"XDG_RUNTIME_DIR set"
		} else {
			"XDG_RUNTIME_DIR unset (systemctl --user needs a user session)"
		},
		Some(r.runtime_dir),
	);
	Ok(())
}
