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
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct WatchRule {
	/// Host path watched for changes, resolved relative to the project base directory.
	pub path: String,
	/// What to do when a watched file changes.
	pub action: WatchAction,
	/// Sync destination inside the container; sync actions are skipped when absent.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub target: Option<String>,
	/// Glob patterns (relative to the base dir) whose matches are excluded from triggering.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub ignore: Vec<String>,
	/// Glob allow-list; when non-empty, only matching paths trigger the rule.
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub include: Vec<String>,
	/// When true, copy `path` to `target` once at startup before watching begins.
	#[serde(default)]
	pub initial_sync: bool,
	/// Command run inside the container for the `sync+exec` action.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub exec: Option<WatchExec>,
	#[serde(flatten, default, skip_serializing_if = "indexmap::IndexMap::is_empty")]
	pub unknown: indexmap::IndexMap<String, serde_yaml::Value>,
}

/// Action triggered by a `develop.watch` rule: `sync`, `rebuild`, `restart`, `sync+restart`, or `sync+exec`.
#[derive(Debug, Clone, Serialize, Default, PartialEq, Eq)]
pub enum WatchAction {
	/// `sync` — copy changed files from `path` into the container at `target`.
	#[default]
	Sync,
	/// `rebuild` — rebuild the service image and recreate the container.
	Rebuild,
	/// `restart` — restart the running container without rebuilding.
	Restart,
	/// `sync+restart` — sync changed files, then restart the container.
	SyncAndRestart,
	/// `sync+exec` — sync changed files, then run the rule's `exec` command in the container.
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
	/// Command and arguments executed in the container (argv form, not shell-parsed).
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
