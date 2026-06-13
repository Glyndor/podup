//! Docker Compose file type definitions.

pub mod build;
pub mod deploy;
pub mod develop;
pub mod env;
pub mod lifecycle;
pub mod network;
pub mod ports;
pub mod primitives;
pub mod resources;
pub mod service;
pub mod volume;

pub use build::*;
pub use deploy::*;
pub use develop::*;
pub use env::*;
pub use lifecycle::*;
pub use network::*;
pub use ports::*;
pub use primitives::*;
pub use resources::*;
pub use service::*;
pub use volume::*;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Top-level `secrets:` entry — defines a named secret available to services.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct SecretConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub file: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub external: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub environment: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub driver_opts: HashMap<String, String>,
	#[serde(default)]
	pub labels: Labels,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub template_driver: Option<String>,
}

/// Top-level `configs:` entry — defines a named config available to services.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ConfigConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub file: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub external: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub content: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub environment: Option<String>,
	#[serde(default)]
	pub labels: Labels,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub driver_opts: HashMap<String, String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub template_driver: Option<String>,
}

/// Root deserialization target for a `docker-compose.yml` file.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ComposeFile {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub version: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub include: Vec<IncludeConfig>,
	#[serde(default)]
	pub services: IndexMap<String, Service>,
	#[serde(default)]
	pub volumes: IndexMap<String, Option<VolumeConfig>>,
	#[serde(default)]
	pub networks: IndexMap<String, Option<NetworkConfig>>,
	#[serde(default)]
	pub secrets: IndexMap<String, SecretConfig>,
	#[serde(default)]
	pub configs: IndexMap<String, ConfigConfig>,
	/// Top-level `x-*` extension fields — preserved and round-tripped via `config` subcommand.
	#[serde(flatten, default, skip_serializing_if = "IndexMap::is_empty")]
	pub extensions: IndexMap<String, serde_yaml::Value>,
}

/// Placeholder substituted for inline secret/config `content:` values when the
/// file is rendered for display.
pub const REDACTED_PLACEHOLDER: &str = "<redacted>";

impl ComposeFile {
	/// Replace every inline `content:` value under `secrets:` and `configs:`
	/// with [`REDACTED_PLACEHOLDER`], so rendering the file for display (the
	/// `config` subcommand) never writes secret material to stdout. References
	/// to a `file:` or `environment:` source are left untouched — they name a
	/// source, they do not embed the value.
	pub fn redact_inline_content(&mut self) {
		for secret in self.secrets.values_mut() {
			if secret.content.is_some() {
				secret.content = Some(REDACTED_PLACEHOLDER.to_string());
			}
		}
		for config in self.configs.values_mut() {
			if config.content.is_some() {
				config.content = Some(REDACTED_PLACEHOLDER.to_string());
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::parse_str;

	#[test]
	fn redacts_inline_secret_and_config_content() {
		let yaml = r#"
secrets:
  inline_secret:
    content: super-secret-value
  file_secret:
    file: ./token.txt
configs:
  inline_config:
    content: embedded-config-body
  env_config:
    environment: CONFIG_FROM_ENV
"#;
		let mut file = parse_str(yaml).unwrap();
		file.redact_inline_content();

		assert_eq!(
			file.secrets["inline_secret"].content.as_deref(),
			Some("<redacted>")
		);
		// A `file:` source carries no embedded value, so nothing to redact.
		assert!(file.secrets["file_secret"].content.is_none());
		assert_eq!(
			file.secrets["file_secret"].file.as_deref(),
			Some("./token.txt")
		);

		assert_eq!(
			file.configs["inline_config"].content.as_deref(),
			Some("<redacted>")
		);
		// An `environment:` source names an env var; the value is not embedded.
		assert!(file.configs["env_config"].content.is_none());
		assert_eq!(
			file.configs["env_config"].environment.as_deref(),
			Some("CONFIG_FROM_ENV")
		);
	}

	#[test]
	fn rendered_config_never_contains_inline_secret_value() {
		let yaml = r#"
secrets:
  db_password:
    content: hunter2-do-not-leak
"#;
		let mut file = parse_str(yaml).unwrap();
		file.redact_inline_content();
		let rendered = serde_yaml::to_string(&file).unwrap();
		assert!(!rendered.contains("hunter2-do-not-leak"));
		assert!(rendered.contains("<redacted>"));
	}
}
