//! The local checks `up` makes before it creates anything.

use std::collections::HashSet;

use crate::compose::dependencies::effective_depends_on;
use crate::compose::types::ComposeFile;
use crate::engine::lifecycle::targets::in_started_set;
use crate::engine::Engine;
use crate::error::{ComposeError, Result};

impl Engine {
	/// Read the environment of every service this `up` brings up, so a broken
	/// or missing `env_file` fails it before a network, a volume, a secret or a
	/// pod exists, as docker compose does when it loads the project.
	///
	/// Nothing is kept: each service reads its environment again when its turn
	/// comes, after what it depends on has run. One thing is left to that second
	/// read: a missing file of a service that has dependencies in this run,
	/// because one of them may be what writes it. That needs the dependencies to
	/// be started: `create` and `--no-start` start none, so there it fails here.
	pub(super) fn check_env_files(
		&self,
		file: &ComposeFile,
		enabled: &HashSet<String>,
		target_set: &Option<HashSet<String>>,
		start: bool,
	) -> Result<()> {
		for (name, service) in &file.services {
			let selected = target_set.as_ref().is_none_or(|set| set.contains(name));
			if !selected || !enabled.contains(name) {
				continue;
			}
			match self.effective_env(service) {
				Ok(_) => {}
				Err(ComposeError::FileNotFound(_))
					if start
						&& effective_depends_on(service, &file.services)
							.service_names()
							.iter()
							.any(|dep| {
								enabled.contains(dep.as_str()) && in_started_set(target_set, dep)
							}) => {}
				Err(e) => return Err(e),
			}
		}
		Ok(())
	}
}
