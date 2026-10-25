//! The `depends_on` conditions of the service or job a `run` was asked for.
//!
//! `run` hands the dependencies of its target to `up`, and `up` waits on the
//! conditions of each service it starts, for the dependencies of those. The
//! target is not started by `up`, so nobody waited on the conditions it states
//! about its own direct dependencies: `publish` with
//! `prepare: {condition: service_completed_successfully}` could be created while
//! `prepare` was still running, or after it had failed.

use crate::compose::dependencies::effective_depends_on;
use crate::compose::types::{ComposeFile, ServiceCondition};
use crate::engine::Engine;
use crate::error::Result;

impl Engine {
	/// Wait for every condition `target` states about its dependencies, before
	/// the one-off container is created. Called only once those dependencies
	/// have been started, so under `--no-deps` there is nothing to wait for.
	/// One declared `required: false` that fails its condition is logged and
	/// does not stop the run.
	pub(super) async fn wait_target_dependencies(
		&self,
		file: &ComposeFile,
		target: &str,
	) -> Result<()> {
		let Some(service) = file.services.get(target) else {
			return Ok(());
		};
		let deps = effective_depends_on(service, &file.services);
		for dep in deps.service_names() {
			let Some(dep_service) = file.services.get(&dep) else {
				continue;
			};
			let container = self.first_replica_name(&dep, dep_service);
			let wait = match deps.condition_for(&dep) {
				ServiceCondition::ServiceStarted => Ok(()),
				ServiceCondition::ServiceHealthy => {
					let disabled = dep_service
						.healthcheck
						.as_ref()
						.is_some_and(|check| check.is_disabled());
					if disabled {
						Ok(())
					} else {
						self.wait_healthy(&container, dep_service, None).await
					}
				}
				ServiceCondition::ServiceCompletedSuccessfully => {
					self.wait_completed(&container).await
				}
			};
			match wait {
				Ok(()) => {}
				Err(error) if !deps.required_for(&dep) => {
					tracing::debug!(
						"optional dependency {dep} (required: false) did not satisfy its condition: {error}"
					);
				}
				Err(error) => return Err(error),
			}
		}
		Ok(())
	}
}

#[cfg(all(test, unix))]
#[path = "deps_wait_tests.rs"]
mod tests;
