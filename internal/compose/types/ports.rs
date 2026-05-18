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
    String(String),
    Number(u16),
}

impl StringOrU16 {
    pub fn as_str_val(&self) -> String {
        match self {
            StringOrU16::String(s) => s.clone(),
            StringOrU16::Number(n) => n.to_string(),
        }
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
    Short(String),
    Long {
        target: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        published: Option<StringOrU16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        protocol: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        host_ip: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        app_protocol: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}
