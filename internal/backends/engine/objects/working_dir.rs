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
	/// equal, and lossless, so two directories never share one label.
	pub(in crate::backends::engine) fn working_dir_label(&self) -> String {
		let dir = std::fs::canonicalize(&self.base_dir).unwrap_or_else(|_| self.base_dir.clone());
		lossless(dir.as_os_str())
	}

	/// The directory the project's resources were created from, when it is not
	/// this one. Networks and volumes carry it (containers too, but they never
	/// outlive both), and either is enough: a `down` without `-v` removes the
	/// networks and keeps the volumes, which are what a later `down -v` would
	/// destroy. Resources from
	/// before the label existed carry no directory and answer `Ok(None)`. A
	/// listing that fails is an error, so a caller that guards a removal can
	/// refuse instead of reading the failure as "nothing recorded".
	async fn project_created_elsewhere(&self) -> Result<Option<String>> {
		let filter = self.project_label_filter_encoded();
		let lists = [
			format!("{API_PREFIX}/networks/json?filters={filter}"),
			// libpod's volume list is filtered here by the label, as the
			// volume sweep of `down` does.
			format!("{API_PREFIX}/volumes/json"),
		];
		let here = self.working_dir_label();
		for path in lists {
			let items = match self.client.get_json::<Vec<serde_json::Value>>(&path).await {
				Ok(items) => items,
				Err(e) if e.is_status(404) => continue,
				Err(e) => return Err(ComposeError::Podman(e)),
			};
			let elsewhere = items.iter().find_map(|item| {
				["/labels", "/Labels", "/Config/Labels"]
					.iter()
					.filter_map(|p| item.pointer(p))
					.filter(|labels| {
						labels.get("podup.project").and_then(|v| v.as_str())
							== Some(self.project.as_str())
					})
					.find_map(|labels| labels.get(WORKING_DIR_LABEL).and_then(|v| v.as_str()))
					.filter(|dir| *dir != here)
					.map(str::to_owned)
			});
			if elsewhere.is_some() {
				return Ok(elsewhere);
			}
		}
		Ok(None)
	}

	/// Warn when the project was created from another directory. Advice only:
	/// a listing that fails says nothing, and the command that follows reports
	/// the daemon problem itself.
	pub(in crate::backends::engine) async fn warn_if_project_used_elsewhere(&self) {
		if let Ok(Some(dir)) = self.project_created_elsewhere().await {
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
	/// the record (created before it existed) is not refused. When the record
	/// cannot be read, the teardown is refused too: it is the check that decides
	/// whether `--force` is needed, so it fails closed.
	pub async fn refuse_if_project_used_elsewhere(&self, action: &str, force: bool) -> Result<()> {
		if force {
			return Ok(());
		}
		match self.project_created_elsewhere().await {
			Ok(Some(dir)) => Err(ComposeError::Unsupported(format!(
				"project '{}' was created from {dir:?}, not from {:?}; `{action}` here would \
				 remove what that directory's project owns. Run it from there, give this one \
				 its own name with -p, or pass --force to do it anyway.",
				self.project,
				self.working_dir_label()
			))),
			Ok(None) => Ok(()),
			Err(e) => Err(ComposeError::Unsupported(format!(
				"`{action}` could not check which directory project '{}' was created from \
				 ({e}); pass --force to do it anyway.",
				self.project
			))),
		}
	}
}

/// A path as label text without losing a byte: valid UTF-8 is kept as it is,
/// and any other byte is written as `\xNN` (a backslash itself as `\\`), so
/// two directories whose names differ only in bytes that are not UTF-8 never
/// end up with the same label.
fn lossless(path: &std::ffi::OsStr) -> String {
	if let Some(text) = path.to_str() {
		return text.to_string();
	}
	#[cfg(unix)]
	{
		use std::os::unix::ffi::OsStrExt;
		let mut out = String::new();
		for chunk in path.as_bytes().utf8_chunks() {
			out.push_str(&chunk.valid().replace('\\', "\\\\"));
			for byte in chunk.invalid() {
				out.push_str(&format!("\\x{byte:02x}"));
			}
		}
		out
	}
	#[cfg(not(unix))]
	{
		path.to_string_lossy().into_owned()
	}
}

// The fake engine listens on a Unix socket.
#[cfg(all(test, unix))]
#[path = "working_dir_tests.rs"]
mod tests;
