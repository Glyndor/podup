use super::super::note_if_idle;
use crate::compose::types::ComposeFile;
use crate::engine::lifecycle::parallel::{filter_levels, first_error, join_bounded};
use crate::engine::Engine;
use crate::error::Result;

impl Engine {
	/// Send a signal to service containers (default: `SIGKILL`).
	///
	/// If `target_services` is empty, all services are signalled.
	pub async fn kill(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		signal: &str,
	) -> Result<()> {
		// Reject an empty/whitespace-only or otherwise invalid signal before
		// issuing any request; libpod would silently treat `signal=` as SIGKILL.
		crate::engine::lifecycle::commands::kill_signal::validate_signal(signal)?;

		let levels = crate::compose::resolve_levels(file)?;
		let levels = filter_levels(file, levels, target_services)?;

		// Prefetch every project container once and group by service (#1363).
		let live_by_service = self.live_project_replicas().await?;

		let acted = std::sync::atomic::AtomicBool::new(false);
		// The board over the containers this signal will reach.
		crate::ui::progress::begin(crate::engine::lifecycle::seed::level_container_resources(
			&levels,
			&live_by_service,
		));
		let outcome = async {
			for level in &levels {
				let futs = level.iter().map(|name| {
					let names = live_by_service.get(name).cloned().unwrap_or_default();
					self.kill_one_service(names, signal, &acted)
				});
				if let Some(e) = first_error(join_bounded(futs).await) {
					return Err(e);
				}
			}
			Ok(())
		}
		.await;
		crate::ui::progress::end();
		outcome?;
		note_if_idle(&acted, "signal");
		Ok(())
	}
}
