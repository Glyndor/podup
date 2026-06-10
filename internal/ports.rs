//! Port mapping parser.
//!
//! Handles all docker-compose port format variants and converts them to
//! bollard's `PortBinding` structures.

use bollard::models::PortBinding;
use std::collections::HashMap;

use crate::compose::types::{PortMapping, StringOrU16};
use crate::error::{ComposeError, Result};

/// A parsed, normalized port binding.
#[derive(Debug, Clone)]
pub struct ParsedPort {
	/// Container port number.
	pub container_port: u16,
	/// Protocol (`tcp`, `udp`, `sctp`).
	pub protocol: String,
	/// Host IP (may be empty to mean all interfaces).
	pub host_ip: String,
	/// Host port (`None` means random / ephemeral; `Some(0)` means runtime-assigned).
	pub host_port: Option<u16>,
}

/// Parse all port mappings in a service, expanding ranges.
pub fn parse_ports(ports: &[PortMapping]) -> Result<Vec<ParsedPort>> {
	let mut result = Vec::new();
	for mapping in ports {
		result.extend(parse_one(mapping)?);
	}
	Ok(result)
}

/// Convert parsed ports into bollard's `PortBindings` and `ExposedPorts` maps.
///
/// Returns `(port_bindings, exposed_ports)`.  Port 0 is encoded as an empty
/// host_port string per the Docker API convention for "auto-assign".
#[allow(clippy::type_complexity)]
pub fn to_bollard(
	ports: &[ParsedPort],
) -> (
	HashMap<String, Option<Vec<PortBinding>>>,
	HashMap<String, HashMap<(), ()>>,
) {
	let mut port_bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
	let mut exposed_ports: HashMap<String, HashMap<(), ()>> = HashMap::new();

	for p in ports {
		let key = format!("{}/{}", p.container_port, p.protocol);
		let host_ip = if p.host_ip.is_empty() {
			"0.0.0.0".to_string()
		} else {
			p.host_ip.clone()
		};
		let host_port = match p.host_port {
			Some(0) => Some(String::new()),
			Some(n) => Some(n.to_string()),
			None => None,
		};
		let binding = PortBinding {
			host_ip: Some(host_ip),
			host_port,
		};
		let bindings = port_bindings
			.entry(key.clone())
			.or_insert_with(|| Some(Vec::new()));
		if let Some(v) = bindings {
			v.push(binding);
		}
		exposed_ports.entry(key).or_default();
	}

	(port_bindings, exposed_ports)
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

fn parse_one(mapping: &PortMapping) -> Result<Vec<ParsedPort>> {
	match mapping {
		PortMapping::Short(s) => parse_short(s),
		PortMapping::Long {
			target,
			published,
			protocol,
			host_ip,
			..
		} => {
			let proto = protocol.clone().unwrap_or_else(|| "tcp".into());
			let hip = host_ip.clone().unwrap_or_default();
			let host_port = published
				.as_ref()
				.map(|p| match p {
					StringOrU16::Number(n) => Ok(*n),
					StringOrU16::String(s) => s.parse::<u16>().map_err(|_| {
						ComposeError::InvalidPort(format!("invalid published port: {s}"))
					}),
				})
				.transpose()?;
			Ok(vec![ParsedPort {
				container_port: *target,
				protocol: proto,
				host_ip: hip,
				host_port,
			}])
		}
	}
}

/// Parse a short-form port string.
///
/// Formats:
/// - `container`
/// - `container/proto`
/// - `host:container`
/// - `host:container/proto`
/// - `ip:host:container` (ip may be IPv4 or `[ipv6]`)
/// - `ip:host:container/proto`
/// - `host_start-host_end:container_start-container_end`
fn parse_short(s: &str) -> Result<Vec<ParsedPort>> {
	// Split off protocol suffix.
	let (rest, proto) = if let Some(idx) = s.rfind('/') {
		(&s[..idx], s[idx + 1..].to_string())
	} else {
		(s, "tcp".to_string())
	};

	// IPv6 form: `[::1]:host:container` or `[::1]:container`.
	if let Some(rest) = rest.strip_prefix('[') {
		let close = rest
			.find(']')
			.ok_or_else(|| ComposeError::InvalidPort(format!("unclosed `[` in {s}")))?;
		let ip = &rest[..close];
		let after = &rest[close + 1..];
		let after = after.strip_prefix(':').unwrap_or(after);
		return parse_with_ip(ip, after, &proto, s);
	}

	// Count colons to determine format.
	let colon_count = rest.chars().filter(|&c| c == ':').count();

	match colon_count {
		0 => {
			// Just container port (possibly a range).
			let ports = expand_port_range(rest)?;
			Ok(ports
				.into_iter()
				.map(|cp| ParsedPort {
					container_port: cp,
					protocol: proto.clone(),
					host_ip: String::new(),
					host_port: None,
				})
				.collect())
		}
		1 => {
			let (left, right) = split_last_colon(rest);
			let host_ports = expand_port_range(left)?;
			let container_ports = expand_port_range(right)?;
			let host_ports = expand_single_host_port(host_ports, container_ports.len(), s)?;
			if host_ports.len() != container_ports.len() {
				return Err(ComposeError::InvalidPort(format!(
					"port range mismatch: {s}"
				)));
			}
			Ok(host_ports
				.into_iter()
				.zip(container_ports)
				.map(|(hp, cp)| ParsedPort {
					container_port: cp,
					protocol: proto.clone(),
					host_ip: String::new(),
					host_port: Some(hp),
				})
				.collect())
		}
		_ => {
			let parts: Vec<&str> = rest.splitn(3, ':').collect();
			if parts.len() < 3 {
				return Err(ComposeError::InvalidPort(format!("invalid port spec: {s}")));
			}
			parse_with_ip(parts[0], &format!("{}:{}", parts[1], parts[2]), &proto, s)
		}
	}
}

/// Parse the `host[:container]` portion when an explicit IP prefix is present.
fn parse_with_ip(ip: &str, after: &str, proto: &str, full: &str) -> Result<Vec<ParsedPort>> {
	if let Some((left, right)) = after.split_once(':') {
		let host_ports = expand_port_range(left)?;
		let container_ports = expand_port_range(right)?;
		let host_ports = expand_single_host_port(host_ports, container_ports.len(), full)?;
		if host_ports.len() != container_ports.len() {
			return Err(ComposeError::InvalidPort(format!(
				"port range mismatch: {full}"
			)));
		}
		Ok(host_ports
			.into_iter()
			.zip(container_ports)
			.map(|(hp, cp)| ParsedPort {
				container_port: cp,
				protocol: proto.to_string(),
				host_ip: ip.to_string(),
				host_port: Some(hp),
			})
			.collect())
	} else {
		let cp: u16 = after
			.parse()
			.map_err(|_| ComposeError::InvalidPort(format!("bad port: {full}")))?;
		Ok(vec![ParsedPort {
			container_port: cp,
			protocol: proto.to_string(),
			host_ip: ip.to_string(),
			host_port: None,
		}])
	}
}

/// Split at the LAST colon (to avoid splitting IPv6 addresses incorrectly).
fn split_last_colon(s: &str) -> (&str, &str) {
	if let Some(idx) = s.rfind(':') {
		(&s[..idx], &s[idx + 1..])
	} else {
		("", s)
	}
}

/// When `host_ports` contains exactly one port and `container_count > 1`, expand
/// the host side to a range starting at `host_ports[0]` (docker-compose semantics
/// for `8080:80-82` → 8080→80, 8081→81, 8082→82).
fn expand_single_host_port(
	host_ports: Vec<u16>,
	container_count: usize,
	spec: &str,
) -> Result<Vec<u16>> {
	if host_ports.len() == 1 && container_count > 1 {
		let start = host_ports[0];
		let end = start
			.checked_add((container_count - 1) as u16)
			.ok_or_else(|| {
				ComposeError::InvalidPort(format!("host port range overflow: {spec}"))
			})?;
		Ok((start..=end).collect())
	} else {
		Ok(host_ports)
	}
}

const MAX_PORT_RANGE: usize = 1024;

/// Expand `start-end` or a single port string.
fn expand_port_range(s: &str) -> Result<Vec<u16>> {
	let s = s.trim();
	if let Some(idx) = s.find('-') {
		let start: u16 = s[..idx]
			.parse()
			.map_err(|_| ComposeError::InvalidPort(format!("bad port: {s}")))?;
		let end: u16 = s[idx + 1..]
			.parse()
			.map_err(|_| ComposeError::InvalidPort(format!("bad port: {s}")))?;
		if start > end {
			return Err(ComposeError::InvalidPort(format!(
				"start > end in range: {s}"
			)));
		}
		let count = (end as usize) - (start as usize) + 1;
		if count > MAX_PORT_RANGE {
			return Err(ComposeError::InvalidPort(format!(
				"port range too large ({count} ports, max {MAX_PORT_RANGE}): {s}"
			)));
		}
		Ok((start..=end).collect())
	} else {
		let p: u16 = s
			.parse()
			.map_err(|_| ComposeError::InvalidPort(format!("bad port: {s}")))?;
		Ok(vec![p])
	}
}
