//! The default that keeps two bridge networks from seeing each other.

use std::collections::HashMap;

/// The netavark option that stops a bridge network from reaching another one.
const ISOLATE_OPT: &str = "isolate";

/// Ask netavark to isolate a bridge network unless the compose file said
/// otherwise.
///
/// Docker isolates its bridge networks from each other and podman does not, so
/// without this a service reaches ports on a neighbouring network that were
/// never published. Measured on 2026-08-20 with the same experiment on both
/// engines: a listener on an unpublished port in network B, and a container in
/// network A dialling its address:
///
/// | engine | result |
/// |---|---|
/// | docker 29.6.2 | refused |
/// | podman 5.7.0 | connected |
///
/// The control matters: inside the *same* network docker connects, so the
/// refusal is isolation rather than a broken dial.
///
/// Also measured on podman 5.7.0 with the option set: traffic within one
/// network still flows (compose keeps working) and outbound internet still
/// works.
///
/// **The option only bites when both networks carry it.** Measured three ways:
/// isolated → isolated is refused, isolated → plain connects, and plain →
/// isolated connects. So this does not harden a network against the world; it
/// makes networks that share the default stop seeing each other. Since podup
/// now sets it on every bridge network it creates, the effect is that two
/// podup projects no longer reach each other's unpublished ports, verified
/// end to end with two projects, while a network created outside podup
/// without the option still can. Being a default rather than opt-in is the
/// whole point: one project opting in would protect nobody.
///
/// A compose file that sets `isolate` in `driver_opts` wins, including setting
/// it to `false`, which is the escape hatch for a project that deliberately
/// spans networks.
pub(crate) fn apply_default_isolation(driver: &str, opts: &mut HashMap<String, String>) {
	if driver != "bridge" {
		return;
	}
	opts.entry(ISOLATE_OPT.to_string())
		.or_insert_with(|| "true".to_string());
}

#[cfg(test)]
#[path = "isolation_tests.rs"]
mod tests;
