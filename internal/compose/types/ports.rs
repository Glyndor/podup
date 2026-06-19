//! Port mapping types used in the `ports:` service field.
//!
//! [`PortMapping`] is either a short-form string (`"8080:80"`) or a long-form
//! struct. [`StringOrU16`] handles the `published` field which the spec allows
//! as either a port number or a quoted string range like `"8080-8090"`.

use serde::{Deserialize, Serialize};

/// A port value that may appear as a bare number or a quoted string in YAML.
///
/// The spec allows `published: 8080` (integer) or `published: "8080"` (string),
/// and also string ranges like `"8080-8090"` for port range mappings.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StringOrU16 {
	/// String form, e.g. a quoted port or range like `"8080-8090"`.
	String(String),
	/// Numeric port form.
	Number(u16),
}

impl StringOrU16 {
	/// Returns the value as a string.
	pub fn as_str_val(&self) -> String {
		match self {
			StringOrU16::String(s) => s.clone(),
			StringOrU16::Number(n) => n.to_string(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::StringOrU16;

	#[test]
	fn string_variant_returns_string() {
		assert_eq!(
			StringOrU16::String("8080-8090".into()).as_str_val(),
			"8080-8090"
		);
	}

	#[test]
	fn number_variant_returns_string_representation() {
		assert_eq!(StringOrU16::Number(443).as_str_val(), "443");
	}
}

/// A single entry in a service's `ports:` list.
///
/// The short form (`"host:container"`, `"ip:host:container/proto"`) is a string
/// and is parsed by [`crate::ports::parse_ports`]. The long form exposes each
/// field individually and supports all spec options including `mode`, `app_protocol`,
/// and per-port naming.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum PortMapping {
	/// Short form: a `host:container[/proto]` string.
	Short(String),
	/// Long form: each port field expressed individually.
	Long {
		/// Container port being exposed.
		target: u16,
		/// Host port (or range) the target is published to.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		published: Option<StringOrU16>,
		/// Transport protocol (`tcp` or `udp`).
		#[serde(default, skip_serializing_if = "Option::is_none")]
		protocol: Option<String>,
		/// Host IP to bind the published port to.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		host_ip: Option<String>,
		/// Publishing mode (`host` or `ingress`).
		#[serde(default, skip_serializing_if = "Option::is_none")]
		mode: Option<String>,
		/// Application-level protocol hint (e.g. `http`).
		#[serde(default, skip_serializing_if = "Option::is_none")]
		app_protocol: Option<String>,
		/// Human-readable name for the port mapping.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		name: Option<String>,
	},
}
