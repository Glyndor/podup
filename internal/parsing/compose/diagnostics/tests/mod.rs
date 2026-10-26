//! Parse-time diagnostics unit tests (split from mod.rs to stay under the
//! per-file line limit).

use crate::parse_str;

fn diagnostics_for(yaml: &str) -> Vec<String> {
	let file = parse_str(yaml).unwrap();
	super::collect(&file)
}

mod build_fields;
mod network_ports;
mod pre_start;
mod secrets_configs;
mod service_fields;
mod unknown_keys;

/// Suppress-port-exposure guard: the RAII handle the CLI installs around the
/// parse step of read-only commands and `--no-warn` live commands, plus the
/// `emit_diagnostic` predicate that consumes it. Tests live here alongside the
/// rest of the parse-time diagnostics suite because the guard is part of that
/// surface and its behaviour is observable only via `emit_diagnostic` (no
/// public `is_suppressed()` predicate).
#[cfg(test)]
mod suppress_port_exposure;
