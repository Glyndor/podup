//! The orphan-container surface: `Engine::remove_orphans`,
//! `Engine::warn_orphans` and the predicate the membership filter reduces to.
//! `Engine::orphan_container_names` reads the project's labelled containers the
//! compose file no longer defines; both entry points share it.

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::Engine;

impl Engine {
	/// Remove containers labelled for this project that are not defined in the current compose file.
	///
	/// Best-effort across every orphan (one that fails to remove must not stop
	/// the rest from being reaped) but the first real failure is remembered and
	/// returned once every orphan has been attempted, so a removal that
	/// genuinely fails does not exit 0 with the orphan left behind (#598). A 404
	/// (already gone) stays an idempotent no-op.
	pub async fn remove_orphans(&self, file: &ComposeFile) -> Result<()> {
		let mut first_err: Option<ComposeError> = None;
		for name in self.orphan_container_names(file).await? {
			tracing::info!("removing orphan container {name}");
			let rm_path = format!("{API_PREFIX}/containers/{}?force=true", urlencoded(&name));
			match self.client.delete_ok(&rm_path).await {
				Ok(()) => {}
				Err(e) if e.is_status(404) => {}
				Err(e) => {
					tracing::debug!("orphan delete {name}: {e}");
					first_err.get_or_insert(ComposeError::Podman(e));
				}
			}
		}
		if let Some(e) = first_err {
			return Err(e);
		}
		Ok(())
	}

	/// Warn (without removing) when this project has orphan containers and
	/// `--remove-orphans` was not given, matching docker compose's `up`.
	pub async fn warn_orphans(&self, file: &ComposeFile) -> Result<()> {
		let orphans = self.orphan_container_names(file).await?;
		if !orphans.is_empty() {
			// Through tracing like every other warning: this printed with no
			// `podup:` prefix and no `warning` label at all, so the one message
			// telling a user their compose file drifted read as stray output.
			tracing::warn!(
				"found orphan container(s) ({}) for this project. If you removed or renamed a \
				 service in your compose file, run with --remove-orphans to remove them.",
				orphans.join(", ")
			);
		}
		Ok(())
	}

	/// The names of this project's labelled containers that the compose file no
	/// longer defines. Shared by `remove_orphans` and `warn_orphans`.
	async fn orphan_container_names(&self, file: &ComposeFile) -> Result<Vec<String>> {
		let path = format!(
			"{API_PREFIX}/containers/json?all=true&filters={}",
			self.project_label_filter_encoded(),
		);

		let running = self
			.client
			.get_json::<Vec<crate::libpod::types::container::ContainerListEntry>>(&path)
			.await
			.map_err(ComposeError::Podman)?;

		// A job's container is the project's too: a job that depends on another
		// job starts it as a container named for it, and `--remove-orphans`
		// must not reap that as a service the file no longer defines.
		let jobs = file.jobs.iter().map(|(n, job)| (n, &job.service));
		let known: std::collections::HashSet<String> = file
			.services
			.iter()
			.chain(jobs)
			.flat_map(|(n, s)| self.replica_names(n, s))
			.collect();

		// A pod's infra container carries the project label and belongs to no
		// service; it lives and dies with the pod, never as an orphan.
		// A one-off `run` of a declared service or job is the declared thing's
		// own container, not one the file forgot: it is told apart by the
		// project's identity labels, not by guessing its name.
		let declared: std::collections::HashSet<&str> = file
			.services
			.keys()
			.chain(file.jobs.keys())
			.map(String::as_str)
			.collect();
		let names: Vec<String> = running
			.iter()
			.filter(|c| !c.is_infra)
			.filter(|c| !self.is_one_off_of(c, &declared))
			.flat_map(|c| c.names.iter())
			.map(|raw| raw.trim_start_matches('/').to_string())
			.collect();
		Ok(filter_orphans(names, &known))
	}
}

impl Engine {
	/// Whether `container` is a one-off `run` of a service or job in `declared`.
	///
	/// A `run` container carries `podup.service` (whose it is) and
	/// `podup.oneoff=true`. Ones created before that label existed are named
	/// `<project>-<service>-run-<number>`, which is also accepted, but only
	/// together with a `podup.service` that is declared.
	fn is_one_off_of(
		&self,
		container: &crate::libpod::types::container::ContainerListEntry,
		declared: &std::collections::HashSet<&str>,
	) -> bool {
		let Some(service) = container.labels.get("podup.service") else {
			return false;
		};
		if !declared.contains(service.as_str()) {
			return false;
		}
		if container.labels.get("podup.oneoff").map(String::as_str) == Some("true") {
			return true;
		}
		let prefix = format!("{}-{service}-run-", self.project);
		container.names.iter().any(|name| {
			name.trim_start_matches('/')
				.strip_prefix(&prefix)
				.is_some_and(|rest| !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()))
		})
	}
}

/// The subset of `names` not present in `known` (the orphan containers). Pure so
/// the membership logic is unit-tested without a live Podman socket.
fn filter_orphans(names: Vec<String>, known: &std::collections::HashSet<String>) -> Vec<String> {
	names.into_iter().filter(|n| !known.contains(n)).collect()
}

#[cfg(test)]
#[path = "orphans_tests.rs"]
mod tests;
