//! Primitive compose field types shared across multiple service keys.
//!
//! [`Command`] — shell string or exec list for `command:`/`entrypoint:`.
//! [`StringOrList`] — single string or list of strings (used in `dns:`, `cap_add:`, etc.).
//! [`Labels`] — list or map form for `labels:`.
//! [`LoggingConfig`] — `logging:` driver and options.
//! [`Sysctls`] — list or map form for `sysctls:`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Container entrypoint / command — either a shell string or exec list.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Command {
	Shell(String),
	Exec(Vec<String>),
}

impl Command {
	pub fn to_exec(&self) -> Vec<String> {
		match self {
			Command::Shell(s) => vec!["sh".into(), "-c".into(), s.clone()],
			Command::Exec(v) => v.clone(),
		}
	}

	pub fn to_argv(&self) -> Vec<String> {
		match self {
			Command::Shell(s) => vec![s.clone()],
			Command::Exec(v) => v.clone(),
		}
	}
}

/// A field that accepts either a single string or a list of strings.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum StringOrList {
	#[default]
	Empty,
	Single(String),
	List(Vec<String>),
}

impl StringOrList {
	pub fn to_list(&self) -> Vec<String> {
		match self {
			StringOrList::Empty => vec![],
			StringOrList::Single(s) => vec![s.clone()],
			StringOrList::List(v) => v.clone(),
		}
	}

	pub fn is_empty(&self) -> bool {
		match self {
			StringOrList::Empty => true,
			StringOrList::Single(s) => s.is_empty(),
			StringOrList::List(v) => v.is_empty(),
		}
	}
}

/// Labels — list or map form.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum Labels {
	#[default]
	Empty,
	List(Vec<String>),
	Map(IndexMap<String, String>),
}

impl Labels {
	pub fn to_map(&self) -> HashMap<String, String> {
		match self {
			Labels::Empty => HashMap::new(),
			Labels::List(list) => list
				.iter()
				.filter_map(|s| {
					let mut parts = s.splitn(2, '=');
					Some((
						parts.next()?.to_string(),
						parts.next().unwrap_or("").to_string(),
					))
				})
				.collect(),
			Labels::Map(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
		}
	}

	pub fn is_empty(&self) -> bool {
		match self {
			Labels::Empty => true,
			Labels::List(v) => v.is_empty(),
			Labels::Map(m) => m.is_empty(),
		}
	}
}

/// `logging:` configuration — driver name and driver-specific options.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct LoggingConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub options: HashMap<String, String>,
}

/// Kernel parameters — list (`["net.ipv4.ip_forward=1"]`) or map form.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum Sysctls {
	#[default]
	Empty,
	List(Vec<String>),
	Map(IndexMap<String, serde_yaml::Value>),
}

impl Sysctls {
	pub fn to_map(&self) -> HashMap<String, String> {
		match self {
			Sysctls::Empty => HashMap::new(),
			Sysctls::List(list) => list
				.iter()
				.filter_map(|s| {
					let mut parts = s.splitn(2, '=');
					let key = parts.next()?.to_string();
					let val = parts.next().unwrap_or("").to_string();
					Some((key, val))
				})
				.collect(),
			Sysctls::Map(m) => m
				.iter()
				.map(|(k, v)| {
					let s = match v {
						serde_yaml::Value::String(s) => s.clone(),
						serde_yaml::Value::Number(n) => n.to_string(),
						serde_yaml::Value::Bool(b) => b.to_string(),
						_ => String::new(),
					};
					(k.clone(), s)
				})
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
	use indexmap::IndexMap;

	// Command

	#[test]
	fn command_shell_to_exec_wraps_in_sh() {
		let cmd = Command::Shell("echo hi".into());
		assert_eq!(cmd.to_exec(), vec!["sh", "-c", "echo hi"]);
	}

	#[test]
	fn command_exec_to_exec_passthrough() {
		let cmd = Command::Exec(vec!["ls".into(), "-la".into()]);
		assert_eq!(cmd.to_exec(), vec!["ls", "-la"]);
	}

	#[test]
	fn command_shell_to_argv_returns_shell_string() {
		let cmd = Command::Shell("echo hi".into());
		assert_eq!(cmd.to_argv(), vec!["echo hi"]);
	}

	#[test]
	fn command_exec_to_argv_passthrough() {
		let cmd = Command::Exec(vec!["ls".into()]);
		assert_eq!(cmd.to_argv(), vec!["ls"]);
	}

	// StringOrList

	#[test]
	fn string_or_list_empty_to_list() {
		assert!(StringOrList::Empty.to_list().is_empty());
	}

	#[test]
	fn string_or_list_single_to_list() {
		assert_eq!(StringOrList::Single("a".into()).to_list(), vec!["a"]);
	}

	#[test]
	fn string_or_list_list_to_list() {
		let s = StringOrList::List(vec!["a".into(), "b".into()]);
		assert_eq!(s.to_list(), vec!["a", "b"]);
	}

	#[test]
	fn string_or_list_empty_is_empty() {
		assert!(StringOrList::Empty.is_empty());
	}

	#[test]
	fn string_or_list_single_empty_string_is_empty() {
		assert!(StringOrList::Single(String::new()).is_empty());
	}

	#[test]
	fn string_or_list_nonempty_single_not_empty() {
		assert!(!StringOrList::Single("x".into()).is_empty());
	}

	// Labels

	#[test]
	fn labels_empty_to_map() {
		assert!(Labels::Empty.to_map().is_empty());
	}

	#[test]
	fn labels_list_parses_key_equals_value() {
		let l = Labels::List(vec!["env=prod".into(), "team=infra".into()]);
		let m = l.to_map();
		assert_eq!(m.get("env").map(|s| s.as_str()), Some("prod"));
		assert_eq!(m.get("team").map(|s| s.as_str()), Some("infra"));
	}

	#[test]
	fn labels_list_key_only_has_empty_value() {
		let l = Labels::List(vec!["bare".into()]);
		let m = l.to_map();
		assert_eq!(m.get("bare").map(|s| s.as_str()), Some(""));
	}

	#[test]
	fn labels_map_to_map() {
		let mut im = IndexMap::new();
		im.insert("k".to_string(), "v".to_string());
		let m = Labels::Map(im).to_map();
		assert_eq!(m.get("k").map(|s| s.as_str()), Some("v"));
	}

	#[test]
	fn labels_is_empty_variants() {
		assert!(Labels::Empty.is_empty());
		assert!(Labels::List(vec![]).is_empty());
		let mut im = IndexMap::new();
		im.insert("x".to_string(), "y".to_string());
		assert!(!Labels::Map(im).is_empty());
	}

	// Sysctls

	#[test]
	fn sysctls_empty_to_map() {
		assert!(Sysctls::Empty.to_map().is_empty());
	}

	#[test]
	fn sysctls_list_parses() {
		let s = Sysctls::List(vec!["net.ipv4.ip_forward=1".into()]);
		let m = s.to_map();
		assert_eq!(m.get("net.ipv4.ip_forward").map(|s| s.as_str()), Some("1"));
	}

	#[test]
	fn sysctls_map_string_value() {
		let mut im = IndexMap::new();
		im.insert(
			"net.core.somaxconn".to_string(),
			serde_yaml::Value::Number(128.into()),
		);
		let m = Sysctls::Map(im).to_map();
		assert_eq!(m.get("net.core.somaxconn").map(|s| s.as_str()), Some("128"));
	}
}
