//! Ordered ephemeral containers, separate from hooks executed in the service.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::compose::pre_start::build_steps;
use crate::compose::types::{ComposeFile, Service};
use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

static NEXT_INIT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
enum Action {
	Run,
	Remove,
}

/// Keep cleanup in the same sequence on start, wait and process failures.
async fn execute_steps<F, Fut>(steps: &[Service], mut perform: F) -> Result<()>
where
	F: FnMut(usize, Action) -> Fut,
	Fut: Future<Output = Result<i64>>,
{
	for index in 0..steps.len() {
		let outcome = perform(index, Action::Run).await;
		let cleanup = perform(index, Action::Remove).await;
		let result = outcome
			.and_then(|code| {
				if code == 0 {
					Ok(())
				} else {
					Err(ComposeError::PreStart(format!("exited with code {code}")))
				}
			})
			.and(cleanup.map(|_| ()));
		result.map_err(|e| ComposeError::PreStart(format!("pre_start step {}: {e}", index + 1)))?;
	}
	Ok(())
}

impl Engine {
	/// Resolve inherited images once, using the same tag as the owning service.
	pub(super) fn pre_start_steps(&self, service_name: &str, service: &Service) -> Vec<Service> {
		let mut steps = build_steps(service);
		for (step, declared) in steps.iter_mut().zip(&service.pre_start) {
			if declared.image.is_none() && declared.build.is_none() {
				step.image = Some(self.service_image_tag(service_name, service));
				step.build = None;
			}
		}
		steps
	}

	/// Reuse a running service without repeating its completed initialization.
	pub(in crate::engine) async fn ensure_initialized_started(
		&self,
		container_name: &str,
		name: &str,
		service: &Service,
		file: &ComposeFile,
	) -> Result<()> {
		if !service.pre_start.is_empty()
			&& self.container_still_running(container_name).await != Some(true)
		{
			self.run_pre_start(container_name, name, service, file)
				.await?;
		}
		self.ensure_started(container_name).await
	}

	/// Finish init work before a caller may create or start the service container.
	pub(in crate::engine) async fn run_pre_start(
		&self,
		container_name: &str,
		service_name: &str,
		service: &Service,
		file: &ComposeFile,
	) -> Result<()> {
		let steps = self.pre_start_steps(service_name, service);
		for (i, step) in steps.iter().enumerate() {
			crate::libpod::validate::pre_validate_spec(
				&format!("{service_name} pre_start step {}", i + 1),
				step,
			)?;
		}
		let serial = NEXT_INIT.fetch_add(1, Ordering::Relaxed);
		let names: Vec<String> = (1..=steps.len())
			.map(|i| {
				format!(
					"{container_name}-pre-start-{}-{serial}-{i}",
					std::process::id()
				)
			})
			.collect();
		let cleanup: Vec<_> = names.iter().map(|_| std::sync::Mutex::new(None)).collect();
		let result = execute_steps(&steps, |index, action| {
			let step = &steps[index];
			let name = &names[index];
			let step_name = format!("{service_name}-pre-start-{}", index + 1);
			let guard = &cleanup[index];
			async move {
				let path = format!("{API_PREFIX}/containers/{}", urlencoded(name));
				match action {
					Action::Run => {
						// The owning service already acquired its image before init work.
						let declared = &service.pre_start[index];
						if declared.image.is_some() || declared.build.is_some() {
							self.acquire_service_image(&step_name, step, file).await?;
						}
						*guard.lock().unwrap() = Some(
							self.client
								.init_cleanup(format!("{path}?force=true&v=true")),
						);
						self.create_and_start(name, &step_name, step, file, true)
							.await?;
						self.client
							.post_empty_json_unbounded::<i64>(&format!(
								"{path}/wait?condition=stopped"
							))
							.await
							.map_err(ComposeError::Podman)
					}
					Action::Remove => {
						match self
							.client
							.delete_ok(&format!("{path}?force=true&v=true"))
							.await
						{
							Ok(()) => {
								if let Some(mut g) = guard.lock().unwrap().take() {
									g.disarm();
								}
								Ok(0)
							}
							Err(e) if e.is_status(404) => {
								if let Some(mut g) = guard.lock().unwrap().take() {
									g.disarm();
								}
								Ok(0)
							}
							Err(e) => Err(ComposeError::Podman(e)),
						}
					}
				}
			}
		})
		.await;
		result.map_err(|e| ComposeError::PreStart(format!("service '{service_name}': {e}")))
	}
}

#[cfg(test)]
#[path = "pre_start_tests.rs"]
mod pre_start_tests;
