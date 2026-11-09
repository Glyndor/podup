#![no_main]

//! Fuzz the port parser, in both the short (`ip:host:container/proto`) and the
//! long (typed fields) compose forms. It must never panic; when it accepts a
//! mapping, every port it yields must serialise back to a short form that
//! parses to the same port, and a range must stay inside its cap.

use libfuzzer_sys::fuzz_target;
use podup::compose::types::{PortMapping, StringOrU16};
use podup::ports::{parse_ports, to_libpod, ParsedPort};

/// `MAX_PORT_RANGE` in `internal/parsing/values/ports.rs`, which is crate-private.
const MAX_PORT_RANGE: usize = 1024;

/// The short form that names exactly `port`.
fn short_form(port: &ParsedPort) -> String {
	let ports = match (port.host_port, port.host_ip.is_empty()) {
		(Some(host), true) => format!("{host}:{}", port.container_port),
		(None, true) => port.container_port.to_string(),
		(Some(host), false) => format!("[{}]:{host}:{}", port.host_ip, port.container_port),
		(None, false) => format!("[{}]:{}", port.host_ip, port.container_port),
	};
	format!("{ports}/{}", port.protocol)
}

fn check_ports(mapping: PortMapping) {
	let Ok(parsed) = parse_ports(std::slice::from_ref(&mapping)) else {
		return;
	};
	assert!(
		parsed.len() <= MAX_PORT_RANGE,
		"a mapping expanded past the range cap"
	);
	let _ = to_libpod(&parsed);
	for port in &parsed {
		assert!(
			matches!(port.protocol.as_str(), "tcp" | "udp" | "sctp"),
			"unsupported protocol accepted: {:?}",
			port.protocol
		);
		// A host IP the long form carries verbatim may hold the `]` the short
		// form uses to end it; that value has no short spelling to compare.
		if port.host_ip.contains(']') {
			continue;
		}
		let spec = short_form(port);
		let again = parse_ports(&[PortMapping::Short(spec.clone())])
			.unwrap_or_else(|e| panic!("{spec:?} came from an accepted port but fails: {e}"));
		assert_eq!(again.len(), 1, "{spec:?} expanded to {} ports", again.len());
		let other = &again[0];
		assert_eq!(
			(
				other.container_port,
				&other.protocol,
				&other.host_ip,
				other.host_port
			),
			(
				port.container_port,
				&port.protocol,
				&port.host_ip,
				port.host_port
			),
			"{spec:?} does not round-trip"
		);
	}
}

fuzz_target!(|data: &[u8]| {
	let Some((&mode, rest)) = data.split_first() else {
		return;
	};
	let Ok(text) = std::str::from_utf8(rest) else {
		return;
	};
	if mode % 3 == 0 {
		check_ports(PortMapping::Short(text.to_string()));
		return;
	}
	// Long form: the target from two bytes, the other fields from lines.
	let mut lines = text.splitn(4, '\n');
	let published = lines.next().unwrap_or("");
	let protocol = lines.next().map(str::to_string);
	let host_ip = lines.next().map(str::to_string);
	let target = u16::from_le_bytes([mode, rest.first().copied().unwrap_or(0)]);
	let published = match published.parse::<u32>() {
		Ok(n) if mode % 3 == 1 => Some(StringOrU16::Number(n)),
		_ if published.is_empty() => None,
		_ => Some(StringOrU16::String(published.to_string())),
	};
	check_ports(PortMapping::Long {
		target,
		published,
		protocol,
		host_ip,
		mode: None,
		app_protocol: None,
		name: None,
	});
});
