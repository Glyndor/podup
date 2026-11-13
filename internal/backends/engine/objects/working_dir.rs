//! Which directory a project was brought up from.
//!
//! A project is addressed by its name only, and two directories can resolve to
//! the same one: two checkouts named `web`, or a `.env` with
//! `COMPOSE_PROJECT_NAME=web` in an unrelated repository. They then share
//! containers, networks and volumes, so a `down -v` in one removes the other's
//! data. Docker behaves the same way. podup records the directory in a label on
//! what it creates, warns when the project is used from another one, and
//! refuses `down -v` and `down --remove-orphans` from there without `--force`,
//! because those remove what the other directory owns.

use crate::engine::Engine;
use crate::error::{ComposeError, Result};
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

	/// The directory the project's networks were created from, when it is not
	/// this one. Networks are made once per project and outlive every
	/// container, so they are the record to read. Resources from before the
	/// label existed carry no directory and answer `None`, and so does a failed
	/// listing: the command that follows reports a daemon problem itself.
	async fn project_created_elsewhere(&self) -> Option<String> {
		let path = format!(
			"{API_PREFIX}/networks/json?filters={}",
			self.project_network_filter_encoded(),
		);
		let networks = self
			.client
			.get_json::<Vec<serde_json::Value>>(&path)
			.await
			.ok()?;
		let here = self.working_dir_label();
		networks.iter().find_map(|net| {
			["/labels", "/Labels"]
				.iter()
				.filter_map(|p| net.pointer(p))
				.find_map(|labels| labels.get(WORKING_DIR_LABEL).and_then(|v| v.as_str()))
				.filter(|dir| *dir != here)
				.map(str::to_owned)
		})
	}

	/// Warn when the project was created from another directory.
	pub(in crate::backends::engine) async fn warn_if_project_used_elsewhere(&self) {
		if let Some(dir) = self.project_created_elsewhere().await {
			tracing::warn!(
				"project '{}' was created from {dir:?}, and this run is from {:?}; \
				 they share containers, networks and volumes, so `down -v` here removes \
				 that project's volumes too. Use -p to give this one its own name.",
				self.project,
				self.working_dir_label()
			);
		}
	}

	/// Refuse a destructive teardown (`down -v`, `down --remove-orphans`) of a
	/// project created from another directory unless `force` is set: it would
	/// remove the other directory's volumes or containers. A project without
	/// the record (created before it existed) is not refused.
	pub async fn refuse_if_project_used_elsewhere(&self, action: &str, force: bool) -> Result<()> {
		if force {
			return Ok(());
		}
		match self.project_created_elsewhere().await {
			Some(dir) => Err(ComposeError::Unsupported(format!(
				"project '{}' was created from {dir:?}, not from {:?}; `{action}` here would \
				 remove what that directory's project owns. Run it from there, give this one \
				 its own name with -p, or pass --force to do it anyway.",
				self.project,
				self.working_dir_label()
			))),
			None => Ok(()),
		}
	}
}

#[cfg(test)]
#[path = "working_dir_tests.rs"]
mod tests;
