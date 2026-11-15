//! `logging.options.tag` when the file names no driver and the host's own
//! default is one that refuses a tag.

use crate::engine::Engine;
use crate::libpod::types::container::LogConfig;

/// Drop `tag` from `log` when `host_driver` is one that refuses it (the same
/// two the `logging:` translation drops it for), and say so. Returns whether it
/// was dropped.
pub(super) fn drop_tag_for_host_driver(
	service_name: &str,
	host_driver: &str,
	log: &mut LogConfig,
) -> bool {
	if !matches!(host_driver, "json-file" | "k8s-file") || log.options.remove("tag").is_none() {
		return false;
	}
	tracing::warn!(
		"{service_name}: logging.options.tag is ignored; the log driver of this host \
		 ({host_driver}) does not support a tag, and the file names no other driver"
	);
	true
}

impl Engine {
	/// For a `logging:` block with a `tag` and no driver, the driver that applies
	/// is the host's, which only the daemon knows. Asked only in that case, and
	/// a daemon that does not answer leaves the tag in place as before.
	pub(in crate::backends::engine::workload::container) async fn drop_tag_for_host_default(
		&self,
		service_name: &str,
		log: Option<&mut LogConfig>,
	) {
		let Some(log) = log.filter(|l| l.driver.is_none() && l.options.contains_key("tag")) else {
			return;
		};
		if let Some(info) = self.host_info().await {
			drop_tag_for_host_driver(service_name, info.log_driver(), log);
		}
	}
}

#[cfg(test)]
#[path = "log_driver_tests.rs"]
mod tests;
