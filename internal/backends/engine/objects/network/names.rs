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
				default_network_name(network, project)
			}
		}
		None => default_network_name(network, project),
	}
}

/// The project-prefixed name a compose network key resolves to when its
/// top-level definition does not override it with `name:` or `external: true`.
///
/// The one place the rule is written down, so a caller that needs to know
/// whether a resolved name was overridden (the config hash) does not have to
/// repeat the `{project}_{key}` shape and risk drifting from it.
pub(in crate::backends::engine) fn default_network_name(network: &str, project: &str) -> String {
	format!("{project}_{network}")
}

#[cfg(test)]
#[path = "names_tests.rs"]
mod tests;
