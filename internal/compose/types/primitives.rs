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
