//! Volume, secret, and config mount types.
//!
//! [`VolumeMount`] covers the `volumes:` list on a service (short and long forms).
//! [`VolumeConfig`] describes top-level named volume definitions.
//! [`ServiceSecretRef`] and [`ServiceConfigRef`] are the per-service `secrets:` /
//! `configs:` attachment points (short = just the name, long = full options).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::Labels;

/// Volume mount type: `volume`, `bind`, `tmpfs`, `npipe`, or `cluster`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VolumeType {
	/// A named or anonymous managed volume.
	Volume,
	/// A host path bind mount.
	Bind,
	/// An in-memory tmpfs mount.
	Tmpfs,
	/// A Windows named-pipe mount.
	Npipe,
	/// A cluster (Swarm) volume.
	Cluster,
}

/// Sub-options for a `bind`-type volume mount.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct BindOptions {
	/// Mount propagation mode (e.g. `rprivate`, `rshared`).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub propagation: Option<String>,
	/// Whether to create the host path if it does not exist.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub create_host_path: Option<bool>,
	/// SELinux relabeling option (`z` shared or `Z` private).
	#[serde(skip_serializing_if = "Option::is_none")]
	pub selinux: Option<String>,
}

/// Sub-options for a `volume`-type mount — nocopy flag and optional driver config.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct VolumeOptions {
	/// Whether to skip copying existing target contents into the volume.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub nocopy: Option<bool>,
	/// Labels applied to the volume.
	#[serde(default)]
	pub labels: Labels,
	/// Volume driver name and options.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver_config: Option<DriverConfig>,
	/// Path within the volume to mount instead of its root.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub subpath: Option<String>,
}

/// Driver name and key-value options nested under `VolumeOptions`.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct DriverConfig {
	/// Volume driver name.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	/// Driver-specific options.
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub options: HashMap<String, String>,
}

/// Sub-options for a `tmpfs`-type mount — size and mode.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TmpfsOptions {
	/// Size of the tmpfs mount in bytes.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub size: Option<u64>,
	/// File mode of the tmpfs mount, stored as the actual permission bits.
	///
	/// A leading-zero string (`0700`) or an explicit `0o700` is interpreted as
	/// octal — the conventional Unix file-mode notation — rather than erroring or
	/// being silently re-interpreted; invalid input is a clear error.
	#[serde(
		default,
		deserialize_with = "deserialize_octal_mode",
		skip_serializing_if = "Option::is_none"
	)]
	pub mode: Option<u32>,
}

/// Deserialize a tmpfs `mode` as octal-aware permission bits.
///
/// YAML numeric scalars are already decoded by the parser (`0o700` → 448), so an
/// integer node is taken as the actual permission bits. A string node — which is
/// how a leading-zero literal like `0700` reaches us (and what previously failed
/// with an opaque untagged error) — is parsed as octal, accepting an optional
/// `0o` prefix and rejecting non-octal digits with a clear message.
fn deserialize_octal_mode<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
	D: serde::Deserializer<'de>,
{
	use serde::de::Error;

	#[derive(Deserialize)]
	#[serde(untagged)]
	enum Raw {
		Int(u32),
		Str(String),
	}

	match Option::<Raw>::deserialize(deserializer)? {
		None => Ok(None),
		Some(Raw::Int(bits)) => Ok(Some(bits)),
		Some(Raw::Str(s)) => {
			let trimmed = s.trim();
			let digits = trimmed
				.strip_prefix("0o")
				.or_else(|| trimmed.strip_prefix("0O"))
				.unwrap_or(trimmed);
			u32::from_str_radix(digits, 8).map(Some).map_err(|_| {
				D::Error::custom(format!(
					"invalid tmpfs mode {s:?}: use octal notation like 0700 or 0o700"
				))
			})
		}
	}
}

/// A volume mount entry — either a short-form string or a long-form typed block.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
pub enum VolumeMount {
	/// Short form: a `source:target[:options]` string.
	Short(String),
	/// Long form: an explicitly typed mount with per-type options.
	Long {
		/// Mount type selecting which options block applies.
		#[serde(rename = "type")]
		volume_type: VolumeType,
		/// Mount source — host path, volume name, or omitted for anonymous/tmpfs.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		source: Option<String>,
		/// Mount target path inside the container.
		target: String,
		/// Whether the mount is read-only.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		read_only: Option<bool>,
		/// Bind-specific options, when `volume_type` is `bind`.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		bind: Option<BindOptions>,
		/// Volume-specific options, when `volume_type` is `volume`.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		volume: Option<VolumeOptions>,
		/// Tmpfs-specific options, when `volume_type` is `tmpfs`.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		tmpfs: Option<TmpfsOptions>,
		/// Mount consistency requirement (a no-op outside Docker Desktop).
		#[serde(default, skip_serializing_if = "Option::is_none")]
		consistency: Option<String>,
	},
}

impl VolumeMount {
	/// Returns the container-side target path of the mount.
	pub fn target(&self) -> &str {
		match self {
			VolumeMount::Short(s) => {
				let parts: Vec<&str> = s.splitn(3, ':').collect();
				if parts.len() >= 2 {
					parts[1]
				} else {
					parts[0]
				}
			}
			VolumeMount::Long { target, .. } => target,
		}
	}
}

/// Named volume definition in the top-level `volumes:` block.
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[non_exhaustive]
pub struct VolumeConfig {
	/// Volume driver name; the runtime default is used if absent.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub driver: Option<String>,
	/// Driver-specific options.
	#[serde(default, skip_serializing_if = "HashMap::is_empty")]
	pub driver_opts: HashMap<String, String>,
	/// Whether the volume is externally managed and not created by podup.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub external: Option<bool>,
	/// Custom volume name overriding the project-prefixed default.
	#[serde(skip_serializing_if = "Option::is_none")]
	pub name: Option<String>,
	/// Labels applied to the volume.
	#[serde(default)]
	pub labels: Labels,
	/// Unrecognized keys preserved verbatim for round-tripping.
	#[serde(flatten, default, skip_serializing_if = "indexmap::IndexMap::is_empty")]
	pub unknown: indexmap::IndexMap<String, serde_yaml::Value>,
}

/// Reference to a named config from a service — short form (name only) or long form with mount target.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ServiceConfigRef {
	/// Short form: the name of a top-level config to mount.
	Short(String),
	/// Long form: a config name with mount target and ownership options.
	Long {
		/// Name of the top-level config to mount.
		source: String,
		/// Mount path inside the container; defaults to `/<source>` if absent.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		target: Option<String>,
		/// Owner UID of the mounted file.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		uid: Option<String>,
		/// Owner GID of the mounted file.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		gid: Option<String>,
		/// File permission mode of the mounted file.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		mode: Option<u32>,
	},
}

impl ServiceConfigRef {
	/// Returns the name of the referenced top-level config.
	pub fn source(&self) -> &str {
		match self {
			ServiceConfigRef::Short(s) => s,
			ServiceConfigRef::Long { source, .. } => source,
		}
	}

	/// Returns the container mount target, if specified.
	pub fn target(&self) -> Option<&str> {
		match self {
			ServiceConfigRef::Short(_) => None,
			ServiceConfigRef::Long { target, .. } => target.as_deref(),
		}
	}
}

/// Reference to a named secret from a service — short form (name only) or long form with mount target.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ServiceSecretRef {
	/// Short form: the name of a top-level secret to mount.
	Short(String),
	/// Long form: a secret name with mount target and ownership options.
	Long {
		/// Name of the top-level secret to mount.
		source: String,
		/// Mount path; defaults to `/run/secrets/<source>` if absent.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		target: Option<String>,
		/// Owner UID of the mounted file.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		uid: Option<String>,
		/// Owner GID of the mounted file.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		gid: Option<String>,
		/// File permission mode of the mounted file.
		#[serde(default, skip_serializing_if = "Option::is_none")]
		mode: Option<u32>,
	},
}

impl ServiceSecretRef {
	/// Returns the name of the referenced top-level secret.
	pub fn source(&self) -> &str {
		match self {
			ServiceSecretRef::Short(s) => s,
			ServiceSecretRef::Long { source, .. } => source,
		}
	}

	/// Returns the container mount target, if specified.
	pub fn target(&self) -> Option<&str> {
		match self {
			ServiceSecretRef::Short(_) => None,
			ServiceSecretRef::Long { target, .. } => target.as_deref(),
		}
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
	use super::*;

	// TmpfsOptions::mode octal parsing

	#[test]
	fn tmpfs_mode_octal_string_is_parsed_as_octal() {
		// A leading-zero literal reaches us as a string and must parse as octal
		// (0700 → 448 permission bits) instead of failing opaquely.
		let opts: TmpfsOptions = serde_yaml::from_str("mode: \"0700\"\n").unwrap();
		assert_eq!(opts.mode, Some(0o700));
		// An explicit 0o prefix in a string also works.
		let opts: TmpfsOptions = serde_yaml::from_str("mode: \"0o755\"\n").unwrap();
		assert_eq!(opts.mode, Some(0o755));
	}

	#[test]
	fn tmpfs_mode_octal_yaml_literal_is_preserved_as_bits() {
		// A YAML `0o700` scalar is decoded to 448 by the parser; we keep those
		// actual permission bits so the renderer's octal format round-trips.
		let opts: TmpfsOptions = serde_yaml::from_str("mode: 0o700\n").unwrap();
		assert_eq!(opts.mode, Some(0o700));
	}

	#[test]
	fn tmpfs_mode_invalid_octal_is_clear_error() {
		// A non-octal string is rejected with a clear error, not silently coerced.
		let err = serde_yaml::from_str::<TmpfsOptions>("mode: \"0o9\"\n").unwrap_err();
		assert!(err.to_string().contains("tmpfs mode"), "got: {err}");
	}

	// VolumeMount::target

	#[test]
	fn volume_mount_short_two_parts_returns_second() {
		let m = VolumeMount::Short("./data:/app/data".to_string());
		assert_eq!(m.target(), "/app/data");
	}

	#[test]
	fn volume_mount_short_three_parts_returns_second() {
		let m = VolumeMount::Short("./data:/app/data:ro".to_string());
		assert_eq!(m.target(), "/app/data");
	}

	#[test]
	fn volume_mount_short_no_colon_returns_whole_string() {
		let m = VolumeMount::Short("/app/data".to_string());
		assert_eq!(m.target(), "/app/data");
	}

	#[test]
	fn volume_mount_long_returns_target_field() {
		let m = VolumeMount::Long {
			volume_type: VolumeType::Bind,
			source: Some("/host/path".to_string()),
			target: "/container/path".to_string(),
			read_only: None,
			bind: None,
			volume: None,
			tmpfs: None,
			consistency: None,
		};
		assert_eq!(m.target(), "/container/path");
	}

	// ServiceConfigRef

	#[test]
	fn config_ref_short_source() {
		let r = ServiceConfigRef::Short("my-config".to_string());
		assert_eq!(r.source(), "my-config");
		assert!(r.target().is_none());
	}

	#[test]
	fn config_ref_long_source_and_target() {
		let r = ServiceConfigRef::Long {
			source: "my-config".to_string(),
			target: Some("/run/configs/my-config".to_string()),
			uid: None,
			gid: None,
			mode: None,
		};
		assert_eq!(r.source(), "my-config");
		assert_eq!(r.target(), Some("/run/configs/my-config"));
	}

	#[test]
	fn config_ref_long_no_target() {
		let r = ServiceConfigRef::Long {
			source: "my-config".to_string(),
			target: None,
			uid: None,
			gid: None,
			mode: None,
		};
		assert!(r.target().is_none());
	}

	// ServiceSecretRef

	#[test]
	fn secret_ref_short_source() {
		let r = ServiceSecretRef::Short("my-secret".to_string());
		assert_eq!(r.source(), "my-secret");
		assert!(r.target().is_none());
	}

	#[test]
	fn secret_ref_long_source_and_target() {
		let r = ServiceSecretRef::Long {
			source: "my-secret".to_string(),
			target: Some("/run/secrets/my-secret".to_string()),
			uid: None,
			gid: None,
			mode: None,
		};
		assert_eq!(r.source(), "my-secret");
		assert_eq!(r.target(), Some("/run/secrets/my-secret"));
	}

	#[test]
	fn secret_ref_long_no_target() {
		let r = ServiceSecretRef::Long {
			source: "my-secret".to_string(),
			target: None,
			uid: None,
			gid: None,
			mode: None,
		};
		assert_eq!(r.source(), "my-secret");
		assert!(r.target().is_none());
	}
}
