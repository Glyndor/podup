use super::super::note_if_idle;
use crate::compose::types::ComposeFile;
use crate::engine::lifecycle::parallel::{first_error, join_bounded};
use crate::engine::lifecycle::parallel::{restart_service_set, retain_levels};
use crate::engine::Engine;
use crate::error::{ComposeError, Result};

impl Engine {
	/// Restart the named service (or all services). Dependents with a `restart` condition in `depends_on` are also restarted.
	pub async fn restart(&self, file: &ComposeFile, service_name: Option<&str>) -> Result<()> {
		let targets: Vec<String> = service_name
			.map(|s| vec![s.to_string()])
			.unwrap_or_default();
		self.restart_with_options(file, &targets, false).await
	}

	/// Restart with options. When `target_services` is empty, all services are
	/// restarted. When `no_deps` is true, dependents with a `depends_on` restart
	/// condition are NOT cascade-restarted.
	pub async fn restart_with_options(
		&self,
		file: &ComposeFile,
		target_services: &[String],
		no_deps: bool,
	) -> Result<()> {
		// Reject unknown target names up front, like the other commands.
		crate::engine::lifecycle::targets::validate_targets(file, target_services)?;
		// The services to restart: the targets plus their restart-cascade
		// dependents (one hop, unless `--no-deps`). `targets` is kept only to
		// label cascade-restarts distinctly in the logs.
		let (restart_set, targets) = restart_service_set(file, target_services, no_deps);

		// Walk dependency levels in order (a dependency restarts before its
		// dependents) but restart every service *within* a level concurrently.
		let levels = retain_levels(crate::compose::resolve_levels(file)?, |n| {
			restart_set.contains(n)
		});

		// Prefetch every project container once and group by service, instead of
		// one container-list round-trip per service (S+1 → 1 for the level walk;
		// #1363). The bulk helper returns all project containers, not just the
		// running ones, so a restart that lands on a stopped replica starts it.
		//
		// The service's replicas, without the one-off containers `run` creates:
		// a one-off shares the service label but is not a replica, and
		// restarting it would run the task a second time.
		let live_by_service = self.live_service_replicas().await?;

		// Attempt every service and surface the first error (in service order) at
		// the end rather than aborting mid-batch and leaving later services
		// unrestarted.
		let acted = std::sync::atomic::AtomicBool::new(false);
		// Seed the board with the containers this restart will walk, in the level
		// order it walks them, before the first `Restarted` line. The set is
		// knowable here and nowhere later: every progress event on this path
		// fires once its own container is already done.
		crate::ui::progress::begin(crate::engine::lifecycle::seed::level_container_resources(
			&levels,
			&live_by_service,
		));
		let outcome = async {
			let mut first_err: Option<ComposeError> = None;
			for level in &levels {
				let futs = level.iter().map(|name| {
					let service = &file.services[name];
					let grace = self.grace_period_secs(service);
					let done = if targets.contains(name) {
						"Restarted"
					} else {
						"Restarted (dependency)"
					};
					let names = live_by_service.get(name).cloned().unwrap_or_default();
					self.restart_one_service(names, grace, done, &acted)
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
		// Close the board on every exit, the way `run_up` does: the region hides
		// the cursor, so an early return through it would leave the terminal
		// without a caret. `end` is idempotent.
		crate::ui::progress::end();
		outcome?;
		note_if_idle(&acted, "restart");
		Ok(())
	}
}
