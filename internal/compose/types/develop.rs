//! Development watch configuration types for the `develop:` service key.
//!
//! [`DevelopConfig`] holds a list of [`WatchRule`]s that drive the file-watch
//! engine. Each rule specifies a host path to monitor, an [`WatchAction`] to
//! take on change (sync, rebuild, restart, or sync+exec), and optional
//! ignore/include glob filters.

use serde::{Deserialize, Serialize};

/// `develop:` service key — holds file-watch rules for the `watch` command.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DevelopConfig {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub watch: Vec<WatchRule>,
}

/// A single `develop.watch` rule: a path to watch, an action to take, and optional ignore filters.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WatchRule {
	pub path: String,
	pub action: WatchAction,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub target: Option<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub ignore: Vec<String>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub include: Vec<String>,
	#[serde(default)]
	pub initial_sync: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub exec: Option<WatchExec>,
}

/// Action triggered by a `develop.watch` rule: `sync`, `rebuild`, `restart`, `sync+restart`, or `sync+exec`.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub enum WatchAction {
	#[default]
	Sync,
	Rebuild,
	Restart,
	SyncAndRestart,
	SyncAndExec,
}

impl<'de> Deserialize<'de> for WatchAction {
	fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
		let s = String::deserialize(d)?;
		match s.as_str() {
			"sync" => Ok(WatchAction::Sync),
			"rebuild" => Ok(WatchAction::Rebuild),
			"restart" => Ok(WatchAction::Restart),
			"sync+restart" => Ok(WatchAction::SyncAndRestart),
			"sync+exec" => Ok(WatchAction::SyncAndExec),
			other => Err(serde::de::Error::custom(format!(
				"unknown watch action: {other}"
			))),
		}
	}
}

/// Exec command run as part of a `sync+exec` watch action.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WatchExec {
	pub command: Vec<String>,
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn watch_action_sync() {
		let a: WatchAction = serde_yaml::from_str("\"sync\"").unwrap();
		assert_eq!(a, WatchAction::Sync);
	}

	#[test]
	fn watch_action_rebuild() {
		let a: WatchAction = serde_yaml::from_str("\"rebuild\"").unwrap();
		assert_eq!(a, WatchAction::Rebuild);
	}

	#[test]
	fn watch_action_restart() {
		let a: WatchAction = serde_yaml::from_str("\"restart\"").unwrap();
		assert_eq!(a, WatchAction::Restart);
	}

	#[test]
	fn watch_action_sync_and_restart() {
		let a: WatchAction = serde_yaml::from_str("\"sync+restart\"").unwrap();
		assert_eq!(a, WatchAction::SyncAndRestart);
	}

	#[test]
	fn watch_action_sync_and_exec() {
		let a: WatchAction = serde_yaml::from_str("\"sync+exec\"").unwrap();
		assert_eq!(a, WatchAction::SyncAndExec);
	}

	#[test]
	fn watch_action_unknown_is_error() {
		assert!(serde_yaml::from_str::<WatchAction>("\"deploy\"").is_err());
	}
}
