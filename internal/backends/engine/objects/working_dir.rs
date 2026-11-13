//! Which directory a project was brought up from.
//!
//! A project is addressed by its name only, and two directories can resolve to
//! the same one: two checkouts named `web`, or a `.env` with
//! `COMPOSE_PROJECT_NAME=web` in an unrelated repository. They then share
//! containers, networks and volumes, so a `down -v` in one removes the other's
//! data. Docker behaves the same way. podup records the directory in a label on
//! what it creates and warns when the project is used from another one; it does
//! not refuse, because sharing a project across directories can be deliberate.

use crate::engine::Engine;
use crate::libpod::API_PREFIX;

/// The label that records the directory a resource was created from.
pub(in crate::backends::engine) const WORKING_DIR_LABEL: &str = "podup.working_dir";

impl Engine {
	/// The project directory as the label records it: canonical, so the same
	/// directory reached through a symlink or with a trailing slash compares
	/// equal.
	pub(in crate::backends::engine) fn working_dir_label(&self) -> String {
		std::fs::canonicalize(&self.base_dir)
			.unwrap_or_else(|_| self.base_dir.clone())
			.to_string_lossy()
			.into_owned()
	}

	/// Warn when the project's networks were created from another directory.
	/// Networks are made once per project and outlive every container, so they
	/// are the record to read. Resources from before the label existed carry no
	/// directory and say nothing, and a failed listing is not an error: this is
	/// advice, and the command that follows reports a daemon problem itself.
	pub(in crate::backends::engine) async fn warn_if_project_used_elsewhere(&self) {
		let path = format!(
			"{API_PREFIX}/networks/json?filters={}",
			self.project_network_filter_encoded(),
		);
		let Ok(networks) = self.client.get_json::<Vec<serde_json::Value>>(&path).await else {
			return;
		};
		let here = self.working_dir_label();
		let elsewhere = networks.iter().find_map(|net| {
			["/labels", "/Labels"]
				.iter()
				.filter_map(|p| net.pointer(p))
				.find_map(|labels| labels.get(WORKING_DIR_LABEL).and_then(|v| v.as_str()))
				.filter(|dir| *dir != here)
				.map(str::to_owned)
		});
		if let Some(dir) = elsewhere {
			tracing::warn!(
				"project '{}' was created from {dir:?}, and this run is from {here:?}; \
				 they share containers, networks and volumes, so `down -v` here removes \
				 that project's volumes too. Use -p to give this one its own name.",
				self.project
			);
		}
	}
}

#[cfg(test)]
#[path = "working_dir_tests.rs"]
mod tests;
