//! What the daemon says about its host, and the limits the host cannot
//! honour.
//!
//! `GET /info` answers with the host's security posture, the log driver it
//! defaults to, and the cgroup layout it runs on. Every container of one
//! `up` meets the same host, so the answer is fetched once and kept: the
//! daemon is asked a single time instead of once per service and per fact.
//!
//! The cgroup layout also decides which limits can be applied at all: a
//! value the host does not support makes the OCI runtime refuse the
//! container, so it is dropped here instead, with a warning naming what
//! went.

use serde::Deserialize;

use crate::engine::Engine;
use crate::libpod::types::container::{LinuxCPU, LinuxMemory, LinuxResources};
use crate::libpod::API_PREFIX;

/// The part of the daemon's `GET /info` answer this crate reads.
#[derive(Deserialize, Default, Clone)]
pub(in crate::backends::engine) struct Info {
	#[serde(default)]
	host: Host,
}

#[derive(Deserialize, Default, Clone)]
struct Host {
	#[serde(default)]
	security: Security,
	#[serde(rename = "logDriver", default)]
	log_driver: String,
	#[serde(rename = "cgroupVersion", default)]
	cgroup_version: String,
	#[serde(rename = "cgroupControllers", default)]
	cgroup_controllers: Vec<String>,
}

#[derive(Deserialize, Default, Clone)]
struct Security {
	#[serde(default)]
	rootless: bool,
}

/// The cgroup facts the daemon reports about its host.
#[derive(Clone, Debug)]
pub(in crate::backends::engine) struct HostCgroups {
	/// `"v1"` or `"v2"`; anything else means the daemon did not say.
	version: String,
	/// The controllers the daemon may delegate to a container.
	controllers: Vec<String>,
}

impl HostCgroups {
	/// Whether the host runs the unified (v2) cgroup hierarchy.
	pub(in crate::backends::engine) fn is_v2(&self) -> bool {
		self.version == "v2"
	}

	/// Whether the host runs the legacy (v1) cgroup hierarchy.
	pub(in crate::backends::engine) fn is_v1(&self) -> bool {
		self.version == "v1"
	}

	/// The controller that carries the block-I/O limits: the unified `io`
	/// controller on cgroups v2, the `blkio` subsystem on cgroups v1.
	pub(in crate::backends::engine) fn block_io_controller(&self) -> &'static str {
		if self.is_v2() {
			"io"
		} else {
			"blkio"
		}
	}

	/// Whether the daemon may delegate the named controller to a container.
	pub(in crate::backends::engine) fn has_controller(&self, name: &str) -> bool {
		self.controllers.iter().any(|c| c == name)
	}
}

impl Info {
	/// Whether the daemon runs rootless.
	pub(in crate::backends::engine) fn is_rootless(&self) -> bool {
		self.host.security.rootless
	}

	/// The log driver the host defaults to when a service names none.
	pub(in crate::backends::engine) fn log_driver(&self) -> &str {
		&self.host.log_driver
	}

	/// The cgroup layout of the host.
	pub(in crate::backends::engine) fn cgroups(&self) -> HostCgroups {
		HostCgroups {
			version: self.host.cgroup_version.clone(),
			controllers: self.host.cgroup_controllers.clone(),
		}
	}
}

impl Engine {
	/// What the daemon says about its host, read on first use and kept for the
	/// rest of the invocation. `None` when the daemon did not answer, which
	/// is remembered too: a host that cannot be asked is not a host whose
	/// limits are known to be unsupported.
	pub(in crate::backends::engine) async fn host_info(&self) -> Option<Info> {
		self.host_info
			.get_or_init(|| async {
				self.client
					.get_json::<Info>(&format!("{API_PREFIX}/info"))
					.await
					.ok()
			})
			.await
			.clone()
	}

	/// The cgroup layout of the host, when the daemon reported one.
	pub(in crate::backends::engine) async fn host_cgroups(&self) -> Option<HostCgroups> {
		self.host_info().await.map(|info| info.cgroups())
	}

	/// Drop from `limits` every value this host cannot honour, and say what
	/// was dropped and why. A limit the host does not support would make the
	/// OCI runtime refuse the container, so it is discarded instead: the
	/// container starts without the limit, the way Docker drops a limit the
	/// host refuses.
	pub(in crate::backends::engine::workload::container) async fn drop_unsupported_limits(
		&self,
		service_name: &str,
		limits: &mut Option<LinuxResources>,
	) {
		let messages = {
			let Some(limits) = limits.as_mut() else {
				return;
			};
			// Nothing to check: no value the host could refuse.
			if !has_host_dependent_limit(limits) {
				return;
			}
			let host = self.host_cgroups().await;
			unsupported_limits(limits, host.as_ref())
		};
		// A block left with nothing in it is not sent at all.
		if limits.as_ref().is_some_and(is_empty_resources) {
			*limits = None;
		}
		for message in messages {
			tracing::warn!("service \"{service_name}\": {message}");
		}
	}
}

/// Whether any value in `limits` depends on what the host supports.
fn has_host_dependent_limit(limits: &LinuxResources) -> bool {
	let memory = limits
		.memory
		.as_ref()
		.is_some_and(|m| m.disable_oom_killer.is_some() || m.swappiness.is_some());
	let cpu = limits
		.cpu
		.as_ref()
		.is_some_and(|c| c.realtime_period.is_some() || c.realtime_runtime.is_some());
	memory || cpu || limits.block_io.is_some()
}

/// Remove from `limits` the values the host cannot honour and describe each
/// one. The messages name the compose field the value came from and what
/// happens to it.
///
/// A host whose layout the daemon did not report keeps every value: they go
/// out as declared, and each one is reported as unchecked rather than
/// silently assumed to work.
fn unsupported_limits(limits: &mut LinuxResources, host: Option<&HostCgroups>) -> Vec<String> {
	let mut messages = Vec::new();
	let fate = match host {
		Some(host) if host.is_v2() => Fate::Drop,
		Some(host) if host.is_v1() => Fate::Keep,
		_ => Fate::Unchecked,
	};

	// The OOM-killer switch, the memory swappiness knob and the realtime
	// scheduling knobs only exist on cgroups v1. On v2 the OCI runtime
	// refuses to start a container that sets any of them; on v1 they are
	// applied and there is nothing to say.
	if let Some(memory) = limits.memory.as_mut() {
		if memory.disable_oom_killer.is_some() {
			match fate {
				Fate::Drop => {
					memory.disable_oom_killer = None;
					messages.push(
						"oom_kill_disable is discarded: cgroups v2 has no OOM-killer switch, \
						 and the OCI runtime refuses to start a container that sets it"
							.to_string(),
					);
				}
				Fate::Unchecked => messages.push(unchecked(
					"oom_kill_disable",
					"on cgroups v2 it makes the container fail to start",
				)),
				Fate::Keep => {}
			}
		}
		if memory.swappiness.is_some() {
			match fate {
				Fate::Drop => {
					memory.swappiness = None;
					messages.push(
						"mem_swappiness is discarded: cgroups v2 has no memory.swappiness \
						 knob, and the OCI runtime refuses to start a container that sets it"
							.to_string(),
					);
				}
				Fate::Unchecked => messages.push(unchecked(
					"mem_swappiness",
					"on cgroups v2 it makes the container fail to start",
				)),
				Fate::Keep => {}
			}
		}
	}
	if let Some(cpu) = limits.cpu.as_mut() {
		if cpu.realtime_period.is_some() {
			match fate {
				Fate::Drop => {
					cpu.realtime_period = None;
					messages.push(
						"cpu_rt_period is discarded: cgroups v2 has no realtime scheduling \
						 period, and the OCI runtime refuses to start a container that sets it"
							.to_string(),
					);
				}
				Fate::Unchecked => messages.push(unchecked(
					"cpu_rt_period",
					"on cgroups v2 it makes the container fail to start",
				)),
				Fate::Keep => {}
			}
		}
		if cpu.realtime_runtime.is_some() {
			match fate {
				Fate::Drop => {
					cpu.realtime_runtime = None;
					messages.push(
						"cpu_rt_runtime is discarded: cgroups v2 has no realtime scheduling \
						 runtime, and the OCI runtime refuses to start a container that sets it"
							.to_string(),
					);
				}
				Fate::Unchecked => messages.push(unchecked(
					"cpu_rt_runtime",
					"on cgroups v2 it makes the container fail to start",
				)),
				Fate::Keep => {}
			}
		}
	}

	// The block-I/O limits need the host's block-I/O controller. Without it
	// the OCI runtime cannot write the weight and the container does not
	// start, so the whole block goes.
	if limits.block_io.is_some() {
		match host {
			Some(host) if !host.has_controller(host.block_io_controller()) => {
				limits.block_io = None;
				messages.push(format!(
					"blkio_config is discarded: this host does not delegate the \"{}\" \
					 cgroup controller, so the OCI runtime cannot apply the block-I/O \
					 limits",
					host.block_io_controller()
				));
			}
			None => {
				messages.push(unchecked(
					"blkio_config",
					"without the block-I/O cgroup controller it makes the container fail to \
					 start",
				));
			}
			_ => {}
		}
	}

	// A block left with nothing in it is not sent: an empty sub-block would
	// reach libpod as a limit the operator never asked for.
	if limits.memory.as_ref().is_some_and(is_empty_memory) {
		limits.memory = None;
	}
	if limits.cpu.as_ref().is_some_and(is_empty_cpu) {
		limits.cpu = None;
	}

	messages
}

/// What happens to a value the host may not be able to honour.
enum Fate {
	/// The host cannot honour it: the value is dropped.
	Drop,
	/// The host honours it: the value stays and nothing is said.
	Keep,
	/// The host's layout is unknown: the value stays, reported as unchecked.
	Unchecked,
}

/// Whether the resource block carries no value at all.
fn is_empty_resources(limits: &LinuxResources) -> bool {
	limits.memory.is_none()
		&& limits.cpu.is_none()
		&& limits.pids.is_none()
		&& limits.block_io.is_none()
		&& limits.devices.is_empty()
}

/// Whether the memory sub-block carries no value at all.
fn is_empty_memory(memory: &LinuxMemory) -> bool {
	memory.limit.is_none()
		&& memory.reservation.is_none()
		&& memory.swap.is_none()
		&& memory.swappiness.is_none()
		&& memory.disable_oom_killer.is_none()
}

/// Whether the CPU sub-block carries no value at all.
fn is_empty_cpu(cpu: &LinuxCPU) -> bool {
	cpu.shares.is_none()
		&& cpu.quota.is_none()
		&& cpu.period.is_none()
		&& cpu.realtime_period.is_none()
		&& cpu.realtime_runtime.is_none()
		&& cpu.cpus.is_none()
}

/// The message for a limit that goes out unchecked because the daemon did not
/// report the host's cgroup layout.
fn unchecked(field: &str, consequence: &str) -> String {
	format!(
		"{field} is sent as declared: the daemon did not report this host's cgroup layout, \
		 and {consequence}"
	)
}

#[cfg(test)]
#[path = "info_tests.rs"]
mod tests;
