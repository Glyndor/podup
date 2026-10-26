use super::super::note_if_idle;
use crate::compose::types::ComposeFile;
use crate::engine::lifecycle::parallel::{filter_levels, first_error, join_bounded};
use crate::engine::Engine;
use crate::error::{ComposeError, Result};

impl Engine {
	/// Start stopped containers.
	///
	/// Services are started in dependency order. If `target_services` is empty,
	/// all services in the compose file are started.
	pub async fn start(&self, file: &ComposeFile, target_services: &[String]) -> Result<()> {
		// Start in dependency order, one level at a time, but start every service
		// within a level concurrently (#757).
		let levels = crate::compose::resolve_levels(file)?;
		let levels = filter_levels(file, levels, target_services)?;

		// Prefetch every project container once and group by service, instead of
		// one container-list round-trip per service (S+1 → 1 for the level walk;
		// #1363). Only act on containers Podman actually has; acting on the
		// static fallback names would POST `/start` to containers that were never
		// created, 404 (swallowed as a no-op), and exit 0 silently, masking that
		// the project was never created. Attempt every live container and
		// aggregate errors rather than aborting on the first.
		let live_by_service = self.live_project_replicas().await?;

		let any_live = std::sync::atomic::AtomicBool::new(false);
		// The board over the containers Podman actually has, in the dependency
		// order the walk below starts them in.
		crate::ui::progress::begin(crate::engine::lifecycle::seed::level_container_resources(
			&levels,
			&live_by_service,
		));
		let outcome = async {
			let mut first_err: Option<ComposeError> = None;
			for level in &levels {
				let futs = level.iter().map(|name| {
					let names = live_by_service.get(name).cloned().unwrap_or_default();
					self.start_one_service(names, &any_live)
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
		// `start`'s flag answers a narrower question than the others': whether any
		// container existed at all, not whether anything changed, so it can name
		// the cause, and the extra clause rides in the verb.
		note_if_idle(&any_live, "start (project not created)");
		Ok(())
	}
}
