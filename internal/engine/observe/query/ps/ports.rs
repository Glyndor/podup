//! The PORTS column renderer for the `ps` table and its JSON counterpart.
//! Untrusted input: `host_port`, `container_port` and `range` come straight
//! off libpod's JSON, so the math is done in `u32` (not `u16`) and pathological
//! values render as the wider real number rather than wrapping. Pure so the
//! formatting is unit-tested.

use crate::libpod::types::container::ContainerPort;

/// Number of consecutive ports a record covers (`range`, at least 1).
pub(super) fn span_len(p: &ContainerPort) -> u16 {
	p.range.filter(|&r| r > 0).unwrap_or(1)
}

/// `base + offset` widened to `u32` before adding. `host_port`,
/// `container_port` and `range` all come straight off libpod's JSON, untrusted
/// input a hostile or buggy daemon could set to any `u16` value, so a plain
/// `u16 + u16` (e.g. `host_port: 65535` with a `range` of 2) can overflow: it
/// wraps silently in a release build and panics under overflow-checks. Doing
/// the addition in `u32` keeps every legitimate port value identical while a
/// pathological one renders as the (larger) real number instead of wrapping.
pub(super) fn widen_add(base: u16, offset: u16) -> u32 {
	u32::from(base) + u32::from(offset)
}

/// Host IP for display: an unset/empty value means all interfaces (`0.0.0.0`),
/// matching Docker/Podman output (libpod commonly omits it).
pub(super) fn display_host_ip(p: &ContainerPort) -> &str {
	p.host_ip
		.as_deref()
		.filter(|s| !s.is_empty())
		.unwrap_or("0.0.0.0")
}

/// Render one port record the way `docker compose ps` does. A collapsed range
/// (`range > 1`) is rendered as `host_start-host_end->cont_start-cont_end` so the
/// whole range is shown rather than only its first mapping.
pub(super) fn format_port_record(p: &ContainerPort) -> String {
	let proto = p
		.protocol
		.as_deref()
		.map(|proto| format!("/{proto}"))
		.unwrap_or_default();
	let host_ip = display_host_ip(p);
	let hp = p.host_port.unwrap_or(0);
	let n = span_len(p);
	if n > 1 {
		format!(
			"{host_ip}:{hp}-{}->{}-{}{proto}",
			widen_add(hp, n - 1),
			p.container_port,
			widen_add(p.container_port, n - 1),
		)
	} else {
		format!("{host_ip}:{hp}->{}{proto}", p.container_port)
	}
}

/// Render a container's published ports as a comma-joined `ps` PORTS cell.
pub(super) fn format_ports(ports: &[ContainerPort]) -> String {
	ports
		.iter()
		.map(format_port_record)
		.collect::<Vec<_>>()
		.join(", ")
}

/// Structured publishers for `ps --format json`, one object per published port
/// (a collapsed range is expanded so every port appears), mirroring the
/// `Publishers` array docker compose emits.
pub(super) fn publishers(ports: &[ContainerPort]) -> Vec<serde_json::Value> {
	let mut out = Vec::new();
	for p in ports {
		let n = span_len(p);
		for i in 0..n {
			out.push(serde_json::json!({
				"URL": display_host_ip(p),
				"TargetPort": widen_add(p.container_port, i),
				"PublishedPort": p.host_port.map(|hp| widen_add(hp, i)),
				"Protocol": p.protocol.as_deref().unwrap_or("tcp"),
			}));
		}
	}
	out
}
