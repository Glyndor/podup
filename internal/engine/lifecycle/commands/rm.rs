use super::note_if_idle;
use crate::compose::types::ComposeFile;
use crate::engine::lifecycle::parallel::{filter_levels, first_error, join_bounded};
use crate::engine::Engine;
use crate::error::{ComposeError, Result};

impl Engine {
	/// Remove stopped service containers.
	///
	/// When `force` is true, running containers are stopped before removal.
	/// Services are removed in reverse dependency order.
	pub async fn rm(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		force: bool,
	) -> Result<()> {
		self.rm_with_options(file, target_services, force, false)
			.await
	}

	/// Remove stopped service containers. `remove_volumes` (`-v/--volumes`) also
	/// removes anonymous volumes attached to each container.
	pub async fn rm_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		force: bool,
		remove_volumes: bool,
	) -> Result<()> {
		// Remove in reverse dependency order, one level at a time, with the
		// per-service removals in a level running concurrently (#757).
		let mut levels = crate::compose::resolve_levels(file)?;
		levels.reverse();
		let levels = filter_levels(file, levels, target_services)?;

		// Prefetch every project container once and group by service (#1363).
		let live_by_service = self.live_project_replicas().await?;

		let acted = std::sync::atomic::AtomicBool::new(false);
		// The board over the containers this removal will walk, in the reversed
		// level order it walks them.
		crate::ui::progress::begin(crate::engine::lifecycle::seed::level_container_resources(
			&levels,
			&live_by_service,
		));
		let outcome = async {
			let mut first_err: Option<ComposeError> = None;
			for level in &levels {
				let futs = level.iter().map(|name| {
					let names = live_by_service.get(name).cloned().unwrap_or_default();
					self.rm_one_service(names, force, remove_volumes, &acted)
				});
				if let Some(e) = first_error(join_bounded(futs).await) {
					first_err.get_or_insert(e);
				}
			}
			match first_err {
				Some(e) => Err(e),
				None => Ok(()),
			}
		}
		.await;
		crate::ui::progress::end();
		outcome?;
		note_if_idle(&acted, "remove");
		Ok(())
	}
}
