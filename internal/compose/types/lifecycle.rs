//! Lifecycle and dependency types: `depends_on:`, `healthcheck:`, `restart:`, and lifecycle hooks.
//!
//! [`DependsOn`] models the service dependency graph — either a simple name list
//! or a map with per-dependency [`ServiceCondition`] semantics. [`HealthCheck`]
//! covers the inline healthcheck definition. [`RestartPolicy`] parses the
//! `restart:` string field. [`LifecycleHook`] is used for `post_start:` and
//! `pre_stop:` hook entries.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::env::EnvVars;
use super::primitives::Command;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum DependsOn {
	#[default]
	Empty,
	List(Vec<String>),
	Map(IndexMap<String, DependsOnCondition>),
}

impl DependsOn {
	pub fn service_names(&self) -> Vec<String> {
		match self {
			DependsOn::Empty => vec![],
			DependsOn::List(v) => v.clone(),
			DependsOn::Map(m) => m.keys().cloned().collect(),
		}
	}

	pub fn condition_for(&self, service: &str) -> ServiceCondition {
		match self {
			DependsOn::Map(m) => m
				.get(service)
				.map(|c| c.condition.clone())
				.unwrap_or(ServiceCondition::ServiceStarted),
			_ => ServiceCondition::ServiceStarted,
		}
	}

	pub fn restart_for(&self, service: &str) -> bool {
		match self {
			DependsOn::Map(m) => m.get(service).and_then(|c| c.restart).unwrap_or(false),
			_ => false,
		}
	}

	pub fn required_for(&self, service: &str) -> bool {
		match self {
			DependsOn::Map(m) => m.get(service).and_then(|c| c.required).unwrap_or(true),
			_ => true,
		}
	}
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DependsOnCondition {
	pub condition: ServiceCondition,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub restart: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub required: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceCondition {
	#[default]
	ServiceStarted,
	ServiceHealthy,
	ServiceCompletedSuccessfully,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct HealthCheck {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub test: Option<Command>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub interval: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub timeout: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub retries: Option<u32>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub start_period: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub start_interval: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub disable: Option<bool>,
}

impl HealthCheck {
	pub fn is_disabled(&self) -> bool {
		if self.disable.unwrap_or(false) {
			return true;
		}
		matches!(&self.test, Some(Command::Exec(v)) if v.len() == 1 && v[0].eq_ignore_ascii_case("NONE"))
	}
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LifecycleHook {
	pub command: Command,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub user: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub privileged: Option<bool>,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub working_dir: Option<String>,
	#[serde(default)]
	pub environment: EnvVars,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum RestartPolicy {
	No,
	Always,
	OnFailure { max_attempts: Option<u32> },
	UnlessStopped,
}

impl<'de> Deserialize<'de> for RestartPolicy {
	fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
	where
		D: serde::Deserializer<'de>,
	{
		let s = String::deserialize(deserializer)?;
		match s.as_str() {
			"no" => Ok(RestartPolicy::No),
			"always" => Ok(RestartPolicy::Always),
			"unless-stopped" => Ok(RestartPolicy::UnlessStopped),
			"on-failure" => Ok(RestartPolicy::OnFailure { max_attempts: None }),
			s if s.starts_with("on-failure:") => {
				let n = s["on-failure:".len()..]
					.parse::<u32>()
					.map_err(serde::de::Error::custom)?;
				Ok(RestartPolicy::OnFailure {
					max_attempts: Some(n),
				})
			}
			other => Err(serde::de::Error::custom(format!(
				"invalid restart policy: {other}"
			))),
		}
	}
}
