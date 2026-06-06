//! Development watch configuration types for the `develop:` service key.
//!
//! [`DevelopConfig`] holds a list of [`WatchRule`]s that drive the file-watch
//! engine. Each rule specifies a host path to monitor, an [`WatchAction`] to
//! take on change (sync, rebuild, restart, or sync+exec), and optional
//! ignore/include glob filters.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// DevelopConfig
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DevelopConfig {
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub watch: Vec<WatchRule>,
}

// ---------------------------------------------------------------------------
// WatchRule
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// WatchAction
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// WatchExec
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WatchExec {
	pub command: Vec<String>,
}
