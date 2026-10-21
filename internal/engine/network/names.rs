//! The compose network key to host network name rule, shared by every caller.

use crate::compose::types::ComposeFile;

/// Resolve the actual network name on the host for a compose network key.
///
/// `pub` so `startup::config_render` (in the binary crate) can call the same
/// function `up` uses, rather than duplicating the resolution rule. The rule
/// is small but every branch (explicit `name:`, `external: true`, the
/// `<project>_<key>` default) has a footgun if a second copy drifts. The
/// crate-level docs (`podup::resolve_network_name`) call out the reuse intent.
pub fn resolve_network_name(network: &str, file: &ComposeFile, project: &str) -> String {
	match file.networks.get(network).and_then(|c| c.as_ref()) {
		Some(cfg) => {
			if let Some(name) = cfg.name.as_deref() {
				name.to_string()
			} else if cfg.external.unwrap_or(false) {
				network.to_string()
			} else {
				format!("{project}_{network}")
			}
		}
		None => format!("{project}_{network}"),
	}
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
