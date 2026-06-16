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

/// `depends_on:` value — absent, a bare list of service names, or a map of service name to condition.
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

/// Long-form per-dependency entry in `depends_on:` — holds the condition and restart flag.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DependsOnCondition {
	pub condition: ServiceCondition,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub restart: Option<bool>,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub required: Option<bool>,
}

/// Condition a dependency must satisfy before the dependent service starts.
#[derive(Debug, Clone, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceCondition {
	#[default]
	ServiceStarted,
	ServiceHealthy,
	ServiceCompletedSuccessfully,
}

/// Inline `healthcheck:` block. `disable: true` overrides any inherited health check.
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
	#[serde(flatten, default, skip_serializing_if = "IndexMap::is_empty")]
	pub unknown: IndexMap<String, serde_yaml::Value>,
}

impl HealthCheck {
	pub fn is_disabled(&self) -> bool {
		if self.disable.unwrap_or(false) {
			return true;
		}
		matches!(&self.test, Some(Command::Exec(v)) if v.len() == 1 && v[0].eq_ignore_ascii_case("NONE"))
	}
}

/// A single `post_start` or `pre_stop` lifecycle hook entry.
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

/// Service-level `restart:` policy — `no`, `always`, `unless-stopped`, or `on-failure` (with optional max-retries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartPolicy {
	No,
	Always,
	OnFailure { max_attempts: Option<u32> },
	UnlessStopped,
}

impl Serialize for RestartPolicy {
	// Emit the compose-spec string form so `config` output round-trips back
	// through `Deserialize` (the derived form would emit `UnlessStopped`).
	fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
	where
		S: serde::Serializer,
	{
		let s = match self {
			RestartPolicy::No => "no".to_string(),
			RestartPolicy::Always => "always".to_string(),
			RestartPolicy::UnlessStopped => "unless-stopped".to_string(),
			RestartPolicy::OnFailure {
				max_attempts: Some(n),
			} => format!("on-failure:{n}"),
			RestartPolicy::OnFailure { max_attempts: None } => "on-failure".to_string(),
		};
		serializer.serialize_str(&s)
	}
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

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;
	use indexmap::IndexMap;

	// DependsOn::service_names

	#[test]
	fn depends_on_empty_has_no_names() {
		assert!(DependsOn::Empty.service_names().is_empty());
	}

	#[test]
	fn restart_policy_serializes_to_compose_string() {
		let ser = |p: &RestartPolicy| serde_yaml::to_string(p).unwrap().trim().to_string();
		assert_eq!(ser(&RestartPolicy::No), "no");
		assert_eq!(ser(&RestartPolicy::Always), "always");
		assert_eq!(ser(&RestartPolicy::UnlessStopped), "unless-stopped");
		assert_eq!(
			ser(&RestartPolicy::OnFailure { max_attempts: None }),
			"on-failure"
		);
		assert_eq!(
			ser(&RestartPolicy::OnFailure {
				max_attempts: Some(5)
			}),
			"on-failure:5"
		);
	}

	#[test]
	fn restart_policy_round_trips_through_yaml() {
		for input in [
			"no",
			"always",
			"unless-stopped",
			"on-failure",
			"on-failure:3",
		] {
			let p: RestartPolicy = serde_yaml::from_str(input).unwrap();
			let out = serde_yaml::to_string(&p).unwrap();
			let reparsed: RestartPolicy = serde_yaml::from_str(&out).unwrap();
			assert_eq!(p, reparsed, "round-trip failed for {input}");
		}
	}

	#[test]
	fn depends_on_list_returns_names() {
		let d = DependsOn::List(vec!["db".into(), "cache".into()]);
		assert_eq!(d.service_names(), vec!["db", "cache"]);
	}

	#[test]
	fn depends_on_map_returns_keys() {
		let mut m = IndexMap::new();
		m.insert(
			"db".to_string(),
			DependsOnCondition {
				condition: ServiceCondition::ServiceHealthy,
				restart: None,
				required: None,
			},
		);
		assert_eq!(DependsOn::Map(m).service_names(), vec!["db"]);
	}

	// DependsOn::condition_for

	#[test]
	fn condition_for_empty_defaults_to_started() {
		assert_eq!(
			DependsOn::Empty.condition_for("db"),
			ServiceCondition::ServiceStarted
		);
	}

	#[test]
	fn condition_for_map_returns_explicit() {
		let mut m = IndexMap::new();
		m.insert(
			"db".to_string(),
			DependsOnCondition {
				condition: ServiceCondition::ServiceHealthy,
				restart: None,
				required: None,
			},
		);
		assert_eq!(
			DependsOn::Map(m).condition_for("db"),
			ServiceCondition::ServiceHealthy
		);
	}

	// DependsOn::restart_for / required_for

	#[test]
	fn restart_for_list_is_false() {
		assert!(!DependsOn::List(vec!["db".into()]).restart_for("db"));
	}

	#[test]
	fn required_for_list_defaults_true() {
		assert!(DependsOn::List(vec!["db".into()]).required_for("db"));
	}

	#[test]
	fn required_for_map_explicit_false() {
		let mut m = IndexMap::new();
		m.insert(
			"db".to_string(),
			DependsOnCondition {
				condition: ServiceCondition::ServiceStarted,
				restart: None,
				required: Some(false),
			},
		);
		assert!(!DependsOn::Map(m).required_for("db"));
	}

	// HealthCheck::is_disabled

	#[test]
	fn healthcheck_disable_true() {
		let hc = HealthCheck {
			disable: Some(true),
			..Default::default()
		};
		assert!(hc.is_disabled());
	}

	#[test]
	fn healthcheck_test_none_exec_disables() {
		let hc = HealthCheck {
			test: Some(Command::Exec(vec!["NONE".to_string()])),
			..Default::default()
		};
		assert!(hc.is_disabled());
	}

	#[test]
	fn healthcheck_real_test_not_disabled() {
		let hc = HealthCheck {
			test: Some(Command::Shell("curl -f http://localhost/".into())),
			..Default::default()
		};
		assert!(!hc.is_disabled());
	}

	// RestartPolicy deserialization

	#[test]
	fn restart_policy_no() {
		let p: RestartPolicy = serde_yaml::from_str("\"no\"").unwrap();
		assert_eq!(p, RestartPolicy::No);
	}

	#[test]
	fn restart_policy_always() {
		let p: RestartPolicy = serde_yaml::from_str("\"always\"").unwrap();
		assert_eq!(p, RestartPolicy::Always);
	}

	#[test]
	fn restart_policy_unless_stopped() {
		let p: RestartPolicy = serde_yaml::from_str("\"unless-stopped\"").unwrap();
		assert_eq!(p, RestartPolicy::UnlessStopped);
	}

	#[test]
	fn restart_policy_on_failure_bare() {
		let p: RestartPolicy = serde_yaml::from_str("\"on-failure\"").unwrap();
		assert_eq!(p, RestartPolicy::OnFailure { max_attempts: None });
	}

	#[test]
	fn restart_policy_on_failure_with_count() {
		let p: RestartPolicy = serde_yaml::from_str("\"on-failure:3\"").unwrap();
		assert_eq!(
			p,
			RestartPolicy::OnFailure {
				max_attempts: Some(3)
			}
		);
	}

	#[test]
	fn restart_policy_invalid_is_error() {
		assert!(serde_yaml::from_str::<RestartPolicy>("\"bogus\"").is_err());
	}
}
