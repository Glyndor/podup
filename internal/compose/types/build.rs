//! Build, include, and extends configuration types.
//!
//! [`BuildConfig`] represents the `build:` key — either a bare context string
//! or a full long-form config. [`IncludeConfig`] and [`ExtendsConfig`] handle
//! the `include:` and `extends:` top-level / per-service directives respectively.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{EnvVars, Labels, StringOrList, UlimitConfig};

/// Compose `include:` directive — either a bare file path or a long-form block.
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

/// Compose `extends:` directive — either a bare service name or a long-form `{service, file}` block.
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

/// Compose `build:` directive — either a bare context path or a full build-config block.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum BuildConfig {
	Context(String),
	Config {
		#[serde(default, skip_serializing_if = "Option::is_none")]
		context: Option<String>,
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
		#[serde(
			default,
			deserialize_with = "super::primitives::deserialize_extra_hosts",
			skip_serializing_if = "Vec::is_empty"
		)]
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
	/// Build context path.
	///
	/// In the long form, `context` is optional per the Compose Spec (v2.22+):
	/// a `build:` block carrying only `dockerfile_inline:` has no context and
	/// defaults to the project directory `.`.
	pub fn context(&self) -> &str {
		match self {
			BuildConfig::Context(ctx) => ctx,
			BuildConfig::Config { context, .. } => context.as_deref().unwrap_or("."),
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

	/// `build.cache_to` — cache export targets (image references).
	pub fn cache_to(&self) -> &[String] {
		match self {
			BuildConfig::Context(_) => &[],
			BuildConfig::Config { cache_to, .. } => cache_to,
		}
	}

	/// `build.ssh` — SSH agent sockets/keys for the build.
	pub fn ssh(&self) -> &[String] {
		match self {
			BuildConfig::Context(_) => &[],
			BuildConfig::Config { ssh, .. } => ssh,
		}
	}

	/// `build.secrets` — names of top-level secrets exposed to the build.
	pub fn secrets(&self) -> &[String] {
		match self {
			BuildConfig::Context(_) => &[],
			BuildConfig::Config { secrets, .. } => secrets,
		}
	}

	/// `build.additional_contexts` — named extra build contexts (`name -> value`).
	pub fn additional_contexts(&self) -> Vec<(String, String)> {
		match self {
			BuildConfig::Context(_) => Vec::new(),
			BuildConfig::Config {
				additional_contexts,
				..
			} => additional_contexts
				.iter()
				.map(|(k, v)| (k.clone(), v.clone()))
				.collect(),
		}
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	// IncludeConfig::paths

	#[test]
	fn include_config_path_returns_single() {
		let c = IncludeConfig::Path("base.yml".into());
		assert_eq!(c.paths(), vec!["base.yml"]);
	}

	#[test]
	fn include_config_long_returns_list() {
		let c = IncludeConfig::Long {
			path: super::super::StringOrList::List(vec!["a.yml".into(), "b.yml".into()]),
			env_file: None,
			project_directory: None,
		};
		assert_eq!(c.paths(), vec!["a.yml", "b.yml"]);
	}

	// ExtendsConfig

	#[test]
	fn extends_service_short_form() {
		let e = ExtendsConfig::Service("base".into());
		assert_eq!(e.service(), "base");
		assert!(e.file().is_none());
	}

	#[test]
	fn extends_config_long_form() {
		let e = ExtendsConfig::Long {
			service: "base".into(),
			file: Some("base.yml".into()),
		};
		assert_eq!(e.service(), "base");
		assert_eq!(e.file(), Some("base.yml"));
	}

	// BuildConfig accessor methods

	#[test]
	fn build_config_context_string() {
		let b = BuildConfig::Context("./app".into());
		assert_eq!(b.context(), "./app");
		assert!(b.dockerfile().is_none());
		assert!(!b.no_cache());
		assert!(!b.pull());
	}

	#[test]
	fn build_config_long_form_context() {
		let b = BuildConfig::Config {
			context: Some("./app".into()),
			dockerfile: Some("Dockerfile.prod".into()),
			dockerfile_inline: None,
			args: EnvVars::Empty,
			target: Some("release".into()),
			cache_from: vec![],
			cache_to: vec![],
			labels: Labels::Empty,
			shm_size: None,
			network: None,
			platforms: vec![],
			additional_contexts: Default::default(),
			no_cache: Some(true),
			pull: None,
			extra_hosts: vec![],
			tags: vec![],
			privileged: None,
			ssh: vec![],
			secrets: vec![],
			ulimits: Default::default(),
			isolation: None,
			entitlements: vec![],
			provenance: None,
			sbom: None,
		};
		assert_eq!(b.context(), "./app");
		assert_eq!(b.dockerfile(), Some("Dockerfile.prod"));
		assert_eq!(b.target(), Some("release"));
		assert!(b.no_cache());
	}

	#[test]
	fn build_with_only_dockerfile_inline_defaults_context_to_dot() {
		// Compose Spec (v2.22+): `build:` may carry only `dockerfile_inline:` with
		// no `context:` — the context then defaults to the project directory `.`.
		let b: BuildConfig = serde_yaml::from_str("dockerfile_inline: |\n  FROM alpine\n").unwrap();
		assert!(matches!(b, BuildConfig::Config { .. }));
		assert_eq!(b.context(), ".");
		assert_eq!(b.dockerfile_inline(), Some("FROM alpine\n"));
	}
}
