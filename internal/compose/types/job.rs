//! The top-level `jobs:` element of the Compose Specification (v2.38).
//!
//! A job is a container that runs to completion. Its definition is a
//! [`Service`] plus the `triggers` that say how it is launched, so the ~60
//! container and workload keys are not declared a second time. `profiles`
//! and `extends` are keys a service already has.
//!
//! Jobs are parsed from raw YAML by `compose::jobs`, not by a derived
//! `Deserialize`: the parser has to name the offending job in its errors, and
//! serde's own error carries no map key.

use serde::Serialize;

use super::Service;

/// One entry of the top-level `jobs:` map.
#[derive(Debug, Clone, Serialize, Default)]
#[non_exhaustive]
pub struct Job {
	/// The container and workload definition, shared with services.
	#[serde(flatten)]
	pub service: Service,
	/// How the job is launched. Required in the file; never empty.
	pub triggers: Triggers,
}

/// The `triggers:` block of a job.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Triggers {
	/// `manual:` allows `podup run` to launch the job. Absent means `true`.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub manual: Option<bool>,
	/// `schedule:` lists crontab schedules. podup validates them and does not
	/// fire them itself.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub schedule: Vec<Schedule>,
}

impl Triggers {
	/// Whether `podup run` may launch the job: `manual` defaults to `true`.
	pub fn allows_manual(&self) -> bool {
		self.manual.unwrap_or(true)
	}
}

/// One element of `triggers.schedule`: a crontab string or an object.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(untagged)]
#[non_exhaustive]
pub enum Schedule {
	/// The short form, a crontab expression.
	Cron(String),
	/// The long form, with the policy keys.
	Detailed(ScheduleSpec),
}

impl Schedule {
	/// The crontab expression, whichever form declared it.
	pub fn cron(&self) -> &str {
		match self {
			Schedule::Cron(cron) => cron,
			Schedule::Detailed(spec) => &spec.cron,
		}
	}
}

/// The object form of a schedule.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScheduleSpec {
	/// Crontab expression. Required.
	pub cron: String,
	/// Time zone name the expression is read in.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub timezone: Option<String>,
	/// What happens when a run is still going at the next fire time.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub concurrency: Option<Concurrency>,
	/// What happens to fires missed while the scheduler was not running.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub missed_fires: Option<MissedFires>,
}

/// `concurrency:` of a schedule. The specification default is `forbid`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Concurrency {
	/// Skip a fire while the previous run is still going.
	Forbid,
	/// Queue the fire until the previous run ends.
	Queue,
}

/// `missed_fires:` of a schedule. The specification default is `one`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MissedFires {
	/// Run once on catch-up, however many fires were missed.
	One,
	/// Run nothing on catch-up.
	Skip,
}
