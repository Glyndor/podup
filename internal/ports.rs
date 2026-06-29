//! Port mapping parser.
//!
//! Handles all docker-compose port format variants and converts them to
//! libpod `PortMapping` structures.

use crate::compose::types::{PortMapping, StringOrU16};
use crate::error::{ComposeError, Result};
use crate::libpod::types::container::PortMapping as LibpodPortMapping;

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

/// Convert parsed ports into libpod `PortMapping` entries for `SpecGenerator`.
pub fn to_libpod(ports: &[ParsedPort]) -> Vec<LibpodPortMapping> {
	ports
		.iter()
		.map(|p| LibpodPortMapping {
			container_port: p.container_port,
			host_port: p.host_port,
			host_ip: if p.host_ip.is_empty() {
				String::new()
			} else {
				p.host_ip.clone()
			},
			protocol: p.protocol.clone(),
			range: None,
		})
		.collect()
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
			validate_protocol(&proto, &format!("{target}/{proto}"))?;
			let hip = host_ip.clone().unwrap_or_default();
			let host_port = published
				.as_ref()
				.map(|p| match p {
					StringOrU16::Number(n) => port_in_range(*n, &n.to_string()),
					StringOrU16::String(s) => {
						let n: u32 = s.parse().map_err(|_| {
							ComposeError::InvalidPort(format!("invalid published port: {s}"))
						})?;
						port_in_range(n, s)
					}
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
	validate_protocol(&proto, s)?;

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
		let cp = parse_port_num(after)
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

pub(crate) const MAX_PORT_RANGE: usize = 1024;

/// The set of transport protocols podman accepts for a published port. A value
/// outside this set is rejected at config time rather than passed verbatim to
/// podman, which would only surface as an opaque create-time error.
const VALID_PROTOCOLS: [&str; 3] = ["tcp", "udp", "sctp"];

/// Validate a port's protocol suffix against the `tcp`/`udp`/`sctp` allow-list.
fn validate_protocol(proto: &str, full: &str) -> Result<()> {
	if VALID_PROTOCOLS.contains(&proto) {
		Ok(())
	} else {
		Err(ComposeError::InvalidPort(format!(
			"unsupported protocol '{proto}' in '{full}'; use tcp, udp, or sctp"
		)))
	}
}

/// Range-check a numeric port and narrow it to `u16`. Surfaces an out-of-range
/// value (e.g. `99999`) as a clear config error so the short and long port forms
/// fail the same way instead of overflowing or leaking a serde enum message.
fn port_in_range(n: u32, label: &str) -> Result<u16> {
	u16::try_from(n)
		.map_err(|_| ComposeError::InvalidPort(format!("port out of range (1-65535): {label}")))
}

/// Parse a single port number, range-checked against 1-65535.
fn parse_port_num(s: &str) -> Result<u16> {
	let n: u32 = s
		.parse()
		.map_err(|_| ComposeError::InvalidPort(format!("bad port: {s}")))?;
	port_in_range(n, s)
}

/// Expand `start-end` or a single port string.
fn expand_port_range(s: &str) -> Result<Vec<u16>> {
	let s = s.trim();
	if let Some(idx) = s.find('-') {
		let start = parse_port_num(&s[..idx])?;
		let end = parse_port_num(&s[idx + 1..])?;
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
		Ok(vec![parse_port_num(s)?])
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use crate::compose::types::{PortMapping, StringOrU16};

	fn short(s: &str) -> PortMapping {
		PortMapping::Short(s.to_string())
	}

	fn parse_one_short(s: &str) -> Vec<ParsedPort> {
		parse_ports(&[short(s)]).unwrap()
	}

	// Container port only

	#[test]
	fn container_port_only() {
		let ports = parse_one_short("80");
		assert_eq!(ports.len(), 1);
		assert_eq!(ports[0].container_port, 80);
		assert_eq!(ports[0].protocol, "tcp");
		assert_eq!(ports[0].host_ip, "");
		assert!(ports[0].host_port.is_none());
	}

	#[test]
	fn container_port_with_explicit_protocol() {
		let ports = parse_one_short("53/udp");
		assert_eq!(ports[0].container_port, 53);
		assert_eq!(ports[0].protocol, "udp");
	}

	// host:container

	#[test]
	fn host_colon_container() {
		let ports = parse_one_short("8080:80");
		assert_eq!(ports[0].container_port, 80);
		assert_eq!(ports[0].host_port, Some(8080));
		assert_eq!(ports[0].host_ip, "");
	}

	// ip:host:container

	#[test]
	fn ip_host_container() {
		let ports = parse_one_short("127.0.0.1:8080:80");
		assert_eq!(ports[0].container_port, 80);
		assert_eq!(ports[0].host_port, Some(8080));
		assert_eq!(ports[0].host_ip, "127.0.0.1");
	}

	#[test]
	fn ipv6_bracketed() {
		let ports = parse_one_short("[::1]:8080:80");
		assert_eq!(ports[0].container_port, 80);
		assert_eq!(ports[0].host_port, Some(8080));
		assert_eq!(ports[0].host_ip, "::1");
	}

	#[test]
	fn ipv6_bracketed_container_only_has_no_host_port() {
		// `[ip]:container` (no published host port) binds the IPv6 address and
		// lets Podman assign the host port — the no-host-port arm of parse_with_ip.
		let ports = parse_one_short("[::1]:80");
		assert_eq!(ports.len(), 1);
		assert_eq!(ports[0].container_port, 80);
		assert_eq!(ports[0].host_ip, "::1");
		assert!(ports[0].host_port.is_none());
	}

	#[test]
	fn malformed_three_part_short_is_error() {
		// More than one colon but a missing third segment is rejected rather than
		// silently mis-parsed.
		assert!(parse_ports(&[short("1.2.3.4:8080:")]).is_err());
	}

	// Range expansion

	#[test]
	fn container_port_range() {
		let ports = parse_one_short("80-82");
		assert_eq!(ports.len(), 3);
		assert_eq!(ports[0].container_port, 80);
		assert_eq!(ports[2].container_port, 82);
	}

	#[test]
	fn host_range_to_container_range() {
		let ports = parse_one_short("8080-8082:80-82");
		assert_eq!(ports.len(), 3);
		assert_eq!(ports[0].host_port, Some(8080));
		assert_eq!(ports[0].container_port, 80);
		assert_eq!(ports[2].host_port, Some(8082));
		assert_eq!(ports[2].container_port, 82);
	}

	#[test]
	fn single_host_expanded_for_container_range() {
		let ports = parse_one_short("8080:80-82");
		assert_eq!(ports.len(), 3);
		assert_eq!(ports[0].host_port, Some(8080));
		assert_eq!(ports[1].host_port, Some(8081));
		assert_eq!(ports[2].host_port, Some(8082));
	}

	// Error cases

	#[test]
	fn range_start_greater_than_end_is_error() {
		assert!(parse_ports(&[short("85-80")]).is_err());
	}

	#[test]
	fn range_too_large_is_error() {
		let big = format!("1-{}", MAX_PORT_RANGE + 1);
		assert!(parse_ports(&[short(&big)]).is_err());
	}

	#[test]
	fn invalid_port_string_is_error() {
		assert!(parse_ports(&[short("abc")]).is_err());
	}

	#[test]
	fn invalid_protocol_suffix_is_rejected() {
		// A protocol outside tcp/udp/sctp is a config error, not something to pass
		// verbatim to podman.
		let err = parse_ports(&[short("80/banana")]).unwrap_err();
		assert!(err.to_string().contains("banana"), "got: {err}");
		assert!(parse_ports(&[short("53/udp")]).is_ok());
		assert!(parse_ports(&[short("9/sctp")]).is_ok());
	}

	#[test]
	fn out_of_range_short_port_reports_range() {
		// 99999 overflows the 1-65535 port space; surface a clear range error
		// rather than a generic parse failure.
		let err = parse_ports(&[short("99999:80")]).unwrap_err();
		assert!(err.to_string().contains("out of range"), "got: {err}");
	}

	#[test]
	fn out_of_range_long_published_reports_range() {
		// The numeric long form deserializes (u32) and is range-checked here, so the
		// user sees the same clear message as the short form instead of a serde
		// untagged-enum error.
		let mapping = PortMapping::Long {
			target: 80,
			published: Some(StringOrU16::Number(99999)),
			protocol: None,
			host_ip: None,
			mode: None,
			app_protocol: None,
			name: None,
		};
		let err = parse_ports(&[mapping]).unwrap_err();
		assert!(err.to_string().contains("out of range"), "got: {err}");
	}

	#[test]
	fn long_form_invalid_protocol_is_rejected() {
		let mapping = PortMapping::Long {
			target: 80,
			published: Some(StringOrU16::Number(8080)),
			protocol: Some("banana".to_string()),
			host_ip: None,
			mode: None,
			app_protocol: None,
			name: None,
		};
		assert!(parse_ports(&[mapping]).is_err());
	}

	#[test]
	fn unclosed_ipv6_bracket_is_error() {
		assert!(parse_ports(&[short("[::1:80")]).is_err());
	}

	#[test]
	fn mismatched_host_and_container_ranges_is_error() {
		// A two-port host range cannot map onto a three-port container range.
		let err = parse_ports(&[short("8080-8081:80-82")]).unwrap_err();
		assert!(err.to_string().contains("mismatch"));
	}

	#[test]
	fn ip_with_mismatched_ranges_is_error() {
		let err = parse_ports(&[short("127.0.0.1:8080-8081:80-82")]).unwrap_err();
		assert!(err.to_string().contains("mismatch"));
	}

	#[test]
	fn single_host_port_range_overflow_is_error() {
		// Expanding a single host port across a container range that would carry
		// it past u16::MAX is rejected rather than wrapping.
		let err = parse_ports(&[short("65535:80-81")]).unwrap_err();
		assert!(err.to_string().contains("overflow"));
	}

	// Long form

	#[test]
	fn long_form_with_published() {
		let mapping = PortMapping::Long {
			target: 80,
			published: Some(StringOrU16::Number(8080)),
			protocol: Some("tcp".to_string()),
			host_ip: Some("0.0.0.0".to_string()),
			mode: None,
			app_protocol: None,
			name: None,
		};
		let ports = parse_ports(&[mapping]).unwrap();
		assert_eq!(ports[0].container_port, 80);
		assert_eq!(ports[0].host_port, Some(8080));
		assert_eq!(ports[0].host_ip, "0.0.0.0");
	}

	#[test]
	fn long_form_no_published_defaults_to_none() {
		let mapping = PortMapping::Long {
			target: 80,
			published: None,
			protocol: None,
			host_ip: None,
			mode: None,
			app_protocol: None,
			name: None,
		};
		let ports = parse_ports(&[mapping]).unwrap();
		assert!(ports[0].host_port.is_none());
		assert_eq!(ports[0].protocol, "tcp");
	}

	// to_libpod

	#[test]
	fn to_libpod_produces_port_mapping() {
		let ports = parse_one_short("8080:80");
		let mappings = to_libpod(&ports);
		assert_eq!(mappings.len(), 1);
		assert_eq!(mappings[0].container_port, 80);
		assert_eq!(mappings[0].host_port, Some(8080));
		assert_eq!(mappings[0].protocol, "tcp");
	}

	#[test]
	fn to_libpod_port_zero_passes_through() {
		let ports = vec![ParsedPort {
			container_port: 80,
			protocol: "tcp".to_string(),
			host_ip: String::new(),
			host_port: Some(0),
		}];
		let mappings = to_libpod(&ports);
		assert_eq!(mappings[0].host_port, Some(0));
	}

	#[test]
	fn to_libpod_no_host_port_is_none() {
		let ports = parse_one_short("80");
		let mappings = to_libpod(&ports);
		assert_eq!(mappings[0].host_port, None);
	}
}
