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

// ---------------------------------------------------------------------------
// Top-level secrets / configs
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// ComposeFile (root)
// ---------------------------------------------------------------------------

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
