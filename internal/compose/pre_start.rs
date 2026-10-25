//! Complete each init specification independently from its owning service.

use super::types::Service;

/// Preserve declaration order and prevent lifecycle lists from recursing into init work.
pub(crate) fn build_steps(service: &Service) -> Vec<Service> {
	service
		.pre_start
		.iter()
		.map(|step| {
			let mut base = service.clone();
			base.pre_start.clear();
			// Network identity belongs to the service, unless a step supplies its own.
			base.ports.clear();
			base.expose.clear();
			base.mac_address = None;
			if let super::types::ServiceNetworks::Map(networks) = &mut base.networks {
				for network in networks.values_mut().flatten() {
					network.ipv4_address = None;
					network.ipv6_address = None;
					network.mac_address = None;
				}
			}
			let mut merged = super::extends::merge_service(base, step.clone());
			// An image and a build are two ways of saying where the
			// container's filesystem comes from, and a step that names one
			// must not inherit the other. A step with its own `image` under
			// a service with `build` was built from the service's context
			// and tagged with the step's image name, so the image it asked
			// for was never pulled; a step with its own `build` under a
			// service with `image` carried the service's image tag while
			// building something else, so nothing was ever built under that
			// name.
			match (step.image.is_some(), step.build.is_some()) {
				(true, false) => merged.build = None,
				(false, true) => merged.image = None,
				_ => {}
			}
			// `network_mode` and `networks` exclude each other, and merging
			// keeps both sides. A step that names one must not inherit the
			// other, or the engine honours `network_mode` and drops the
			// networks the step asked for without saying so.
			if !step.networks.names().is_empty() && step.network_mode.is_none() {
				merged.network_mode = None;
			} else if step.network_mode.is_some() && step.networks.names().is_empty() {
				merged.networks = super::types::ServiceNetworks::Empty;
			}
			merged.pre_start.clear();
			merged.post_start.clear();
			merged.pre_stop.clear();
			merged.container_name = None;
			// An init process must finish once, even if the service restarts forever.
			merged.restart = Some(super::types::RestartPolicy::No);
			if let Some(deploy) = merged.deploy.as_mut() {
				deploy.restart_policy = None;
			}
			merged
		})
		.collect()
}

/// The effective init steps of a service, each paired with the name the
/// per-container passes report it under.
///
/// The effective step is the container that runs: the service's own fields
/// completed by the step's, minus the ones that belong to the service alone
/// (ports, expose and static addresses). The audit walks these, because it
/// reports on containers and not on text: a step that asks for
/// `privileged: true` runs privileged whether or not the service did.
pub fn effective_steps(service_name: &str, service: &Service) -> Vec<(String, Service)> {
	build_steps(service)
		.into_iter()
		.enumerate()
		.map(|(index, step)| (step_name(service_name, index), step))
		.collect()
}

/// The init steps a service declares, each paired with the name the
/// per-container passes report it under.
///
/// The declared step is the operator's own text, before the service fills
/// the unset fields in. The parse-time diagnostics and the config
/// validation walk these, so a warning or an error names the place the
/// operator wrote it and never repeats what the service already reported.
pub(crate) fn declared_steps<'a>(
	service_name: &str,
	service: &'a Service,
) -> Vec<(String, &'a Service)> {
	service
		.pre_start
		.iter()
		.enumerate()
		.map(|(index, step)| (step_name(service_name, index), step))
		.collect()
}

/// The name a per-container pass reports an init step under:
/// `<service>/pre_start[i]`, with `i` counting from 1, so a finding about an
/// init container is distinguishable from a finding about the service it
/// belongs to.
fn step_name(service_name: &str, index: usize) -> String {
	format!("{service_name}/pre_start[{}]", index + 1)
}

#[cfg(test)]
#[path = "pre_start_inheritance_tests.rs"]
mod pre_start_tests;

#[cfg(test)]
#[path = "pre_start_network_tests.rs"]
mod pre_start_network_tests;
