//! Resource limit and device types shared across service and deploy configuration.
//!
//! [`UlimitConfig`] maps to the `ulimits:` map — either a single value (soft==hard)
//! or an explicit soft/hard pair. [`BlkioConfig`] covers block I/O weight and rate
//! limits. [`GpuSpec`] handles the `gpus:` shorthand (`"all"` or a count).

use serde::{Deserialize, Serialize};

/// `ulimits:` entry — either a single integer (soft == hard) or an explicit `{soft, hard}` pair.
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

/// `blkio_config:` service field — controls I/O weight and per-device rate limits.
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

/// Per-device I/O weight entry under `blkio_config.weight_device`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BlkioWeightDevice {
	pub path: String,
	pub weight: u16,
}

/// Per-device rate limit under `blkio_config` — used for both bps and iops limits.
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

/// Top-level service `gpus:` field. The spec allows the `all`/`N` shorthand and
/// a list of device-reservation objects (same shape as
/// `deploy.resources.reservations.devices`); both forms are accepted so a valid
/// compose file never fails to parse.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum GpuSpec {
	Named(String),
	Count(u32),
	Devices(Vec<super::deploy::DeviceReservation>),
}

impl GpuSpec {
	/// -1 = all; positive = exact count. The device-list form is treated as
	/// "all" since it is not mapped to CDI from this field (use
	/// `deploy.resources.reservations.devices` for that).
	pub fn to_count(&self) -> i64 {
		match self {
			GpuSpec::Named(_) | GpuSpec::Devices(_) => -1,
			GpuSpec::Count(n) => *n as i64,
		}
	}
}
