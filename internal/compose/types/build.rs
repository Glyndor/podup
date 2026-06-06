//! Build, include, and extends configuration types.
//!
//! [`BuildConfig`] represents the `build:` key — either a bare context string
//! or a full long-form config. [`IncludeConfig`] and [`ExtendsConfig`] handle
//! the `include:` and `extends:` top-level / per-service directives respectively.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{EnvVars, Labels, StringOrList, UlimitConfig};

// ---------------------------------------------------------------------------
// IncludeConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum IncludeConfig {
	Path(String),
	Long {
		path: StringOrList,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		env_file: Option<StringOrList>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		project_directory: Option<String>,
	},
}

impl IncludeConfig {
	pub fn paths(&self) -> Vec<String> {
		match self {
			IncludeConfig::Path(p) => vec![p.clone()],
			IncludeConfig::Long { path, .. } => path.to_list(),
		}
	}
}

// ---------------------------------------------------------------------------
// ExtendsConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ExtendsConfig {
	Service(String),
	Long {
		service: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		file: Option<String>,
	},
}

impl ExtendsConfig {
	pub fn service(&self) -> &str {
		match self {
			ExtendsConfig::Service(s) => s,
			ExtendsConfig::Long { service, .. } => service,
		}
	}

	pub fn file(&self) -> Option<&str> {
		match self {
			ExtendsConfig::Service(_) => None,
			ExtendsConfig::Long { file, .. } => file.as_deref(),
		}
	}
}

// ---------------------------------------------------------------------------
// BuildConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum BuildConfig {
	Context(String),
	Config {
		context: String,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		dockerfile: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		dockerfile_inline: Option<String>,
		#[serde(default)]
		args: EnvVars,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		target: Option<String>,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		cache_from: Vec<String>,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		cache_to: Vec<String>,
		#[serde(default)]
		labels: Labels,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		shm_size: Option<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		network: Option<String>,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		platforms: Vec<String>,
		#[serde(default, skip_serializing_if = "HashMap::is_empty")]
		additional_contexts: HashMap<String, String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		no_cache: Option<bool>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		pull: Option<bool>,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		extra_hosts: Vec<String>,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		tags: Vec<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		privileged: Option<bool>,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		ssh: Vec<String>,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		secrets: Vec<String>,
		#[serde(default, skip_serializing_if = "IndexMap::is_empty")]
		ulimits: IndexMap<String, UlimitConfig>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		isolation: Option<String>,
		#[serde(default, skip_serializing_if = "Vec::is_empty")]
		entitlements: Vec<String>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		provenance: Option<serde_yaml::Value>,
		#[serde(default, skip_serializing_if = "Option::is_none")]
		sbom: Option<bool>,
	},
}

impl BuildConfig {
	pub fn context(&self) -> &str {
		match self {
			BuildConfig::Context(ctx) => ctx,
			BuildConfig::Config { context, .. } => context,
		}
	}

	pub fn dockerfile(&self) -> Option<&str> {
		match self {
			BuildConfig::Context(_) => None,
			BuildConfig::Config { dockerfile, .. } => dockerfile.as_deref(),
		}
	}

	pub fn args(&self) -> EnvVars {
		match self {
			BuildConfig::Context(_) => EnvVars::Empty,
			BuildConfig::Config { args, .. } => args.clone(),
		}
	}

	pub fn target(&self) -> Option<&str> {
		match self {
			BuildConfig::Context(_) => None,
			BuildConfig::Config { target, .. } => target.as_deref(),
		}
	}

	pub fn no_cache(&self) -> bool {
		match self {
			BuildConfig::Context(_) => false,
			BuildConfig::Config { no_cache, .. } => no_cache.unwrap_or(false),
		}
	}

	pub fn pull(&self) -> bool {
		match self {
			BuildConfig::Context(_) => false,
			BuildConfig::Config { pull, .. } => pull.unwrap_or(false),
		}
	}

	pub fn shm_size(&self) -> Option<&str> {
		match self {
			BuildConfig::Context(_) => None,
			BuildConfig::Config { shm_size, .. } => shm_size.as_deref(),
		}
	}

	pub fn extra_hosts(&self) -> &[String] {
		match self {
			BuildConfig::Context(_) => &[],
			BuildConfig::Config { extra_hosts, .. } => extra_hosts,
		}
	}

	pub fn tags(&self) -> &[String] {
		match self {
			BuildConfig::Context(_) => &[],
			BuildConfig::Config { tags, .. } => tags,
		}
	}

	pub fn cache_from(&self) -> &[String] {
		match self {
			BuildConfig::Context(_) => &[],
			BuildConfig::Config { cache_from, .. } => cache_from,
		}
	}

	pub fn dockerfile_inline(&self) -> Option<&str> {
		match self {
			BuildConfig::Context(_) => None,
			BuildConfig::Config {
				dockerfile_inline, ..
			} => dockerfile_inline.as_deref(),
		}
	}
}
