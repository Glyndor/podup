//! Quadlet export of the `jobs:` section: a schedule becomes a systemd timer.
//!
//! A job with at least one schedule that has a faithful timer produces:
//!
//! - one `<project>-<job>.container`, written by the same code as a service's
//!   unit, with `Type=oneshot` and without `Restart=` or `[Install]`, because
//!   a timer starts it and nothing enables it;
//! - one `<project>-<job>-<n>.timer` per exportable `schedule` entry, all of
//!   them pointing at that service.
//!
//! A schedule entry is exported only when the timer fires at the same times
//! and under the same policy as the compose file says. Each one that is not
//! gets a warning naming the job, the entry and the reason, and no unit.
//! `cron` and `on_calendar` hold the conversion, `timer` the unit text and
//! the checks it repeats, `schedule` the decision per entry.

mod cron;
mod field;
mod on_calendar;
mod schedule;
mod stale;
mod timer;

pub use self::stale::stale_timers;

use indexmap::IndexMap;

use self::schedule::{plan, Plan};
use self::timer::{is_fixed_offset_zone, is_safe_unit_name, job_stem, timer_unit, TimerSpec};
use super::unit::{build_unit, emits_build_unit, job_container_unit, UnitContext};
use super::{QuadletOutput, QuadletUnit};
use crate::compose::types::{ComposeFile, DependsOn, Job, Service};
use crate::error::sanitize_name;

/// A job with the timers that are written for it.
struct Exported<'a> {
	name: &'a str,
	job: &'a Job,
	/// 1-based entry position and its plan.
	timers: Vec<(usize, Plan)>,
}

/// Append the units of every exportable job of `file` to `out`, and a warning
/// for everything that is not exported.
pub(super) fn append_job_units(file: &ComposeFile, ctx: &UnitContext<'_>, out: &mut QuadletOutput) {
	if file.jobs.is_empty() {
		return;
	}
	let mut manual_only: Vec<&str> = Vec::new();
	let mut exported: Vec<Exported<'_>> = Vec::new();
	for (name, job) in &file.jobs {
		if job.triggers.schedule.is_empty() {
			manual_only.push(name);
			continue;
		}
		let block = job_blocker(name, job, ctx);
		let mut timers = Vec::new();
		for (index, entry) in job.triggers.schedule.iter().enumerate() {
			let position = index + 1;
			match block.clone().map_or_else(|| plan(entry), Err) {
				Ok(plan) => timers.push((position, plan)),
				Err(reason) => out.warnings.push(format!(
					"job '{}': schedule entry {position} was not exported: {reason}",
					sanitize_name(name)
				)),
			}
		}
		if !timers.is_empty() {
			exported.push(Exported { name, job, timers });
		}
	}
	out.warnings
		.extend(super::warnings::manual_only_jobs(&manual_only));

	let mut services: IndexMap<String, Service> = ctx.services.clone();
	for entry in &exported {
		services.insert(entry.name.to_string(), entry.job.service.clone());
	}
	let job_ctx = UnitContext {
		services: &services,
		..*ctx
	};
	let mut timer_count = 0;
	for entry in &exported {
		timer_count += export_job(entry, ctx, &job_ctx, out);
	}
	if timer_count > 0 {
		out.warnings
			.push(super::warnings::timers_need_systemd_dir(timer_count));
	}
}

/// Why no schedule of this job can be exported, if there is such a reason.
fn job_blocker(name: &str, job: &Job, ctx: &UnitContext<'_>) -> Option<String> {
	if ctx.pod_mode {
		return Some(
			"with `x-podman-pod: true` the services share one pod and a job is not part of it"
				.into(),
		);
	}
	if job.service.image.is_none() && !emits_build_unit(&job.service) {
		return Some(
			"the job has no image to run (an inline Dockerfile has no Quadlet form)".into(),
		);
	}
	let stem = job_stem(ctx.project, name);
	let longest_timer = format!("{stem}-{}.timer", job.triggers.schedule.len());
	for file_name in [format!("{stem}.container"), longest_timer] {
		if !is_safe_unit_name(&file_name) {
			return Some(
				"its unit file name is not a plain systemd unit name (it has `..` or is too long)"
					.into(),
			);
		}
	}
	None
}

/// Write the units of one job; returns how many timers were written.
fn export_job(
	entry: &Exported<'_>,
	ctx: &UnitContext<'_>,
	job_ctx: &UnitContext<'_>,
	out: &mut QuadletOutput,
) -> usize {
	let name = entry.name;
	let label = sanitize_name(name);
	let mut service = entry.job.service.clone();
	let restart_set = service.restart.is_some()
		|| service
			.deploy
			.as_ref()
			.is_some_and(|d| d.restart_policy.is_some());
	if restart_set {
		out.warnings.push(format!(
			"job '{label}': its restart policy is not exported; a scheduled run is one execution \
			 and is not retried"
		));
	}
	for dropped in drop_unexported_dependencies(&mut service, job_ctx.services) {
		out.warnings.push(format!(
			"job '{label}': `depends_on` job '{}' is not ordered in systemd because that job \
			 has no exported schedule and so no unit",
			sanitize_name(&dropped)
		));
	}
	let build = build_unit(
		name,
		ctx.project,
		&service,
		ctx.base_dir,
		ctx.secrets,
		&mut out.warnings,
	);
	let container = job_container_unit(name, &service, job_ctx, &mut out.warnings);
	// The timer starts the job with `--replace`, so a container name that
	// another generated unit uses would have that unit's container removed.
	if let Some(clash) = clashing_container_name(&container, &out.units) {
		for (position, _) in &entry.timers {
			out.warnings.push(format!(
				"job '{label}': schedule entry {position} was not exported: its container name \
				 '{}' is also used by {}, and the timer would replace that container; give one \
				 of them a different `container_name`",
				clash.0, clash.1
			));
		}
		return 0;
	}
	let timers: Vec<_> = entry
		.timers
		.iter()
		.map(|(position, plan)| {
			let spec = TimerSpec {
				project: ctx.project,
				job: name,
				index: *position,
				calendar: &plan.calendar,
				timezone: plan.timezone.as_deref(),
				persistent: plan.persistent,
			};
			(*position, plan, timer_unit(&spec))
		})
		.collect();
	out.units.extend(build);
	out.units.push(container);
	let mut written = 0;
	let mut follows_dst = false;
	for (position, plan, result) in timers {
		match result {
			Ok(unit) => {
				out.units.push(unit);
				written += 1;
				follows_dst |= !plan.timezone.as_deref().is_some_and(is_fixed_offset_zone);
				if plan.never_fires {
					out.warnings.push(format!(
						"job '{label}': schedule entry {position} is exported but never fires: \
						 no chosen day of the month exists in any chosen month"
					));
				}
			}
			Err(reason) => out.warnings.push(format!(
				"job '{label}': schedule entry {position} was not exported: {reason}"
			)),
		}
	}
	if follows_dst {
		out.warnings.push(format!(
			"job '{label}': systemd does not treat daylight saving time changes as cron does. In \
			 a zone with summer time a run set for the skipped spring hour does not happen that \
			 day, and a run set for the repeated autumn hour can be missed; `Persistent=` does \
			 not make it up. Use `timezone: UTC` (or an `Etc/` zone) for exact times"
		));
	}
	written
}

/// The container name of `unit` and the file of an earlier `.container` unit
/// that uses the same one, if there is one.
fn clashing_container_name(
	unit: &QuadletUnit,
	earlier: &[QuadletUnit],
) -> Option<(String, String)> {
	let name = container_name(&unit.contents)?;
	earlier
		.iter()
		.filter(|other| other.filename.ends_with(".container"))
		.find(|other| container_name(&other.contents) == Some(name))
		.map(|other| (name.to_string(), other.filename.clone()))
}

fn container_name(contents: &str) -> Option<&str> {
	contents
		.lines()
		.find_map(|line| line.strip_prefix("ContainerName="))
}

/// Remove the `depends_on` entries that name a job without an exported unit,
/// and return their names. `known` holds the services and the exported jobs, so
/// an entry for either stays.
fn drop_unexported_dependencies(
	service: &mut Service,
	known: &IndexMap<String, Service>,
) -> Vec<String> {
	let keep = |name: &str| known.contains_key(name);
	let dropped: Vec<String> = service
		.depends_on
		.service_names()
		.into_iter()
		.filter(|name| !keep(name))
		.collect();
	match &mut service.depends_on {
		DependsOn::Empty => {}
		DependsOn::List(names) => names.retain(|name| keep(name)),
		DependsOn::Map(map) => map.retain(|name, _| keep(name)),
	}
	dropped
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "hostile_tests.rs"]
mod hostile_tests;

#[cfg(test)]
#[path = "policy_tests.rs"]
mod policy_tests;

#[cfg(test)]
#[path = "systemd_tests.rs"]
mod systemd_tests;
