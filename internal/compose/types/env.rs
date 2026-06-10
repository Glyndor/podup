//! Environment variable and env-file types for the `environment:` and `env_file:` service fields.
//!
//! [`EnvVars`] accepts list or map form. [`EnvFile`] accepts a single path, a list of paths,
//! or a list of long-form [`EnvFileEntry`] objects with optional `required:` and `format:` fields.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Environment variables as a list (`["KEY=VAL"]`) or map (`{KEY: VAL}`).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum EnvVars {
	#[default]
	Empty,
	List(Vec<String>),
	Map(IndexMap<String, Option<serde_yaml::Value>>),
}

impl EnvVars {
	pub fn to_map(&self) -> HashMap<String, Option<String>> {
		match self {
			EnvVars::Empty => HashMap::new(),
			EnvVars::List(list) => list
				.iter()
				.filter_map(|s| {
					let mut parts = s.splitn(2, '=');
					let key = parts.next()?.to_string();
					let val = parts.next().map(|v| v.to_string());
					Some((key, val))
				})
				.collect(),
			EnvVars::Map(map) => map
				.iter()
				.map(|(k, v)| {
					let val = v.as_ref().and_then(|v| match v {
						serde_yaml::Value::String(s) => Some(s.clone()),
						serde_yaml::Value::Number(n) => Some(n.to_string()),
						serde_yaml::Value::Bool(b) => Some(b.to_string()),
						serde_yaml::Value::Null => None,
						_ => None,
					});
					(k.clone(), val)
				})
				.collect(),
		}
	}

	pub fn is_empty(&self) -> bool {
		match self {
			EnvVars::Empty => true,
			EnvVars::List(v) => v.is_empty(),
			EnvVars::Map(m) => m.is_empty(),
		}
	}
}

/// One entry in an `env_file:` list — either a bare path or a long-form object.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum EnvFileEntry {
	Path(String),
	Config {
		path: String,
		#[serde(skip_serializing_if = "Option::is_none")]
		required: Option<bool>,
		#[serde(skip_serializing_if = "Option::is_none")]
		format: Option<String>,
	},
}

impl EnvFileEntry {
	pub fn path(&self) -> &str {
		match self {
			EnvFileEntry::Path(p) => p,
			EnvFileEntry::Config { path, .. } => path,
		}
	}

	/// `true` by default — missing file is an error unless `required: false`.
	pub fn required(&self) -> bool {
		match self {
			EnvFileEntry::Path(_) => true,
			EnvFileEntry::Config { required, .. } => required.unwrap_or(true),
		}
	}
}

/// `env_file:` field — single path, list of paths, or list of long-form objects.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(untagged)]
pub enum EnvFile {
	#[default]
	Empty,
	Single(EnvFileEntry),
	List(Vec<EnvFileEntry>),
}

impl EnvFile {
	pub fn to_entries(&self) -> Vec<EnvFileEntry> {
		match self {
			EnvFile::Empty => vec![],
			EnvFile::Single(e) => vec![e.clone()],
			EnvFile::List(v) => v.clone(),
		}
	}

	/// Return just the paths (strips `required` / `format` info).
	pub fn to_list(&self) -> Vec<String> {
		self.to_entries()
			.into_iter()
			.map(|e| e.path().to_string())
			.collect()
	}

	pub fn is_empty(&self) -> bool {
		match self {
			EnvFile::Empty => true,
			EnvFile::Single(_) => false,
			EnvFile::List(v) => v.is_empty(),
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

	// EnvVars::to_map

	#[test]
	fn env_vars_empty_to_map() {
		assert!(EnvVars::Empty.to_map().is_empty());
	}

	#[test]
	fn env_vars_list_key_equals_value() {
		let e = EnvVars::List(vec!["FOO=bar".into(), "BAZ=qux".into()]);
		let m = e.to_map();
		assert_eq!(m.get("FOO"), Some(&Some("bar".to_string())));
		assert_eq!(m.get("BAZ"), Some(&Some("qux".to_string())));
	}

	#[test]
	fn env_vars_list_key_only_has_none_value() {
		let e = EnvVars::List(vec!["HOST_VAR".into()]);
		let m = e.to_map();
		assert_eq!(m.get("HOST_VAR"), Some(&None));
	}

	#[test]
	fn env_vars_map_string_value() {
		let mut im = IndexMap::new();
		im.insert(
			"PORT".to_string(),
			Some(serde_yaml::Value::Number(8080.into())),
		);
		let m = EnvVars::Map(im).to_map();
		assert_eq!(m.get("PORT"), Some(&Some("8080".to_string())));
	}

	#[test]
	fn env_vars_map_null_value_is_none() {
		let mut im = IndexMap::new();
		im.insert("X".to_string(), Some(serde_yaml::Value::Null));
		let m = EnvVars::Map(im).to_map();
		assert_eq!(m.get("X"), Some(&None));
	}

	#[test]
	fn env_vars_is_empty_variants() {
		assert!(EnvVars::Empty.is_empty());
		assert!(EnvVars::List(vec![]).is_empty());
		assert!(!EnvVars::List(vec!["X=1".into()]).is_empty());
	}

	// EnvFileEntry

	#[test]
	fn env_file_entry_path_required_true() {
		let e = EnvFileEntry::Path(".env".into());
		assert_eq!(e.path(), ".env");
		assert!(e.required());
	}

	#[test]
	fn env_file_entry_config_not_required() {
		let e = EnvFileEntry::Config {
			path: ".env.optional".into(),
			required: Some(false),
			format: None,
		};
		assert!(!e.required());
	}

	#[test]
	fn env_file_entry_config_missing_required_defaults_true() {
		let e = EnvFileEntry::Config {
			path: ".env".into(),
			required: None,
			format: None,
		};
		assert!(e.required());
	}

	// EnvFile::to_entries / is_empty

	#[test]
	fn env_file_empty_to_entries() {
		assert!(EnvFile::Empty.to_entries().is_empty());
	}

	#[test]
	fn env_file_single_to_entries() {
		let e = EnvFile::Single(EnvFileEntry::Path(".env".into()));
		assert_eq!(e.to_entries().len(), 1);
	}

	#[test]
	fn env_file_list_to_list_returns_paths() {
		let e = EnvFile::List(vec![
			EnvFileEntry::Path(".env".into()),
			EnvFileEntry::Path(".env.local".into()),
		]);
		assert_eq!(e.to_list(), vec![".env", ".env.local"]);
	}

	#[test]
	fn env_file_is_empty_variants() {
		assert!(EnvFile::Empty.is_empty());
		assert!(EnvFile::List(vec![]).is_empty());
		assert!(!EnvFile::Single(EnvFileEntry::Path(".env".into())).is_empty());
	}
}
