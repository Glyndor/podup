//! Resource limit and device types shared across service and deploy configuration.
//!
//! [`UlimitConfig`] maps to the `ulimits:` map — either a single value (soft==hard)
//! or an explicit soft/hard pair. [`BlkioConfig`] covers block I/O weight and rate
//! limits. [`GpuSpec`] handles the `gpus:` shorthand (`"all"` or a count).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum UlimitConfig {
	Single(i64),
	Pair { soft: i64, hard: i64 },
}

impl UlimitConfig {
	pub fn soft(&self) -> i64 {
		match self {
			UlimitConfig::Single(n) => *n,
			UlimitConfig::Pair { soft, .. } => *soft,
		}
	}

	pub fn hard(&self) -> i64 {
		match self {
			UlimitConfig::Single(n) => *n,
			UlimitConfig::Pair { hard, .. } => *hard,
		}
	}
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BlkioConfig {
	#[serde(skip_serializing_if = "Option::is_none")]
	pub weight: Option<u16>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub weight_device: Vec<BlkioWeightDevice>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub device_read_bps: Vec<BlkioRateDevice>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub device_write_bps: Vec<BlkioRateDevice>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub device_read_iops: Vec<BlkioRateDevice>,
	#[serde(default, skip_serializing_if = "Vec::is_empty")]
	pub device_write_iops: Vec<BlkioRateDevice>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BlkioWeightDevice {
	pub path: String,
	pub weight: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BlkioRateDevice {
	pub path: String,
	pub rate: serde_yaml::Value,
}

impl BlkioRateDevice {
	/// Return rate as bytes/second (or IOPS as a plain integer).
	pub fn rate_value(&self) -> i64 {
		match &self.rate {
			serde_yaml::Value::Number(n) => n.as_i64().unwrap_or(0),
			serde_yaml::Value::String(s) => crate::size::parse_memory(s).unwrap_or(0),
			_ => 0,
		}
	}
}

/// `gpus: all` or `gpus: 2` top-level service field.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum GpuSpec {
	Named(String),
	Count(u32),
}

impl GpuSpec {
	/// -1 = all; positive = exact count.
	pub fn to_count(&self) -> i64 {
		match self {
			GpuSpec::Named(_) => -1,
			GpuSpec::Count(n) => *n as i64,
		}
	}
}
