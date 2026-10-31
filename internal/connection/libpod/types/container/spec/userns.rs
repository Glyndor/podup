//! I encode the storage options required for automatic user namespaces.

use serde::Serialize;

use super::Namespace;

/// I mirror the untagged Go IDMappingOptions fields used by automatic mappings.
#[derive(Serialize)]
pub struct IdMappingOptions {
	#[serde(rename = "HostUIDMapping")]
	host_uid_mapping: bool,
	#[serde(rename = "HostGIDMapping")]
	host_gid_mapping: bool,
	#[serde(rename = "AutoUserNs")]
	auto_user_ns: bool,
	#[serde(rename = "AutoUserNsOpts")]
	auto_user_ns_opts: AutoUserNsOptions,
}

#[derive(Default, Serialize)]
struct AutoUserNsOptions {
	#[serde(rename = "Size")]
	size: u32,
}

impl Namespace {
	/// I supply auto allocation options; Podman computes keep-id and nomap maps.
	pub fn id_mappings(&self) -> Result<Option<IdMappingOptions>, String> {
		if self.nsmode != "auto" {
			return Ok(None);
		}
		let mut options = AutoUserNsOptions::default();
		if let Some(value) = &self.value {
			for option in value.split(',') {
				let Some(size) = option.strip_prefix("size=") else {
					return Err(format!(
						"unsupported auto user namespace option {option:?}; expected size=<uint32>"
					));
				};
				options.size = size
					.parse()
					.ok()
					.filter(|_| size.bytes().all(|byte| byte.is_ascii_digit()))
					.ok_or_else(|| {
						format!("invalid auto user namespace size {size:?}; expected uint32")
					})?;
			}
		}
		Ok(Some(IdMappingOptions {
			host_uid_mapping: false,
			host_gid_mapping: false,
			auto_user_ns: true,
			auto_user_ns_opts: options,
		}))
	}
}
