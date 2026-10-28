//! The `podup autostart install` options bag: which unit to render, whether to
//! start it now, whether to print a plan instead of acting, and the optional
//! auto-update timer pair.

/// Options for [`install`](crate::autostart::install).
///
/// `#[non_exhaustive]`: see [`ServiceUnitOpts`](crate::autostart::ServiceUnitOpts), same reason, same construction
/// pattern.
#[non_exhaustive]
#[derive(Default)]
pub struct InstallOptions {
	/// The unit to render and install.
	pub unit: super::super::service::ServiceUnitOpts,
	/// Install the unit but do not `enable --now` it (no immediate start).
	pub no_start: bool,
	/// Print the unit and the actions that would run, but change nothing.
	pub dry_run: bool,
	/// When set, also render and install the sibling `<unit>.update.service`
	/// oneshot and `<unit>.update.timer`. The string is the `OnCalendar=`
	/// word (`hourly`/`daily`/`weekly`).
	pub auto_update_interval: Option<String>,
}

impl InstallOptions {
	/// Install `unit` with the default flags (start it, really write it).
	pub fn new(unit: super::super::service::ServiceUnitOpts) -> Self {
		Self {
			unit,
			no_start: false,
			dry_run: false,
			auto_update_interval: None,
		}
	}

	/// Install the unit but do not `enable --now` it. Builder-style.
	pub fn with_no_start(mut self, no_start: bool) -> Self {
		self.no_start = no_start;
		self
	}

	/// Print what would happen and change nothing. Builder-style.
	pub fn with_dry_run(mut self, dry_run: bool) -> Self {
		self.dry_run = dry_run;
		self
	}

	/// Install the auto-update timer pair alongside the main unit, with the
	/// given `OnCalendar=` word (`hourly`/`daily`/`weekly`). Builder-style.
	pub fn with_auto_update_interval(mut self, interval: Option<String>) -> Self {
		self.auto_update_interval = interval;
		self
	}
}
