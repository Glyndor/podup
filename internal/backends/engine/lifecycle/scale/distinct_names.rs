//! Two services must not resolve to the same container name.
//!
//! A name is syntactically valid whoever holds it, so the first `up` of
//! `web` with a replica `proj-web-2` and a `worker` that fixes
//! `container_name: proj-web-2` created one of them and then failed or replaced
//! it, depending on the order. Checked before anything is created, over the
//! replica names every enabled service will have at the scale in effect.

use std::collections::{HashMap, HashSet};

use crate::compose::types::ComposeFile;
use crate::engine::Engine;
use crate::error::{ComposeError, Result};

impl Engine {
	pub(in crate::backends::engine::lifecycle) fn check_distinct_container_names(
		&self,
		file: &ComposeFile,
		enabled: &HashSet<String>,
	) -> Result<()> {
		let mut owner: HashMap<String, &str> = HashMap::new();
		let mut names: Vec<&String> = enabled.iter().collect();
		names.sort();
		for name in names {
			let Some(service) = file.services.get(name) else {
				continue;
			};
			// The count is bounded here too, before a hostile `scale:` can make a
			// list of names that long.
			let replicas = self.resolve_replicas(name, service);
			super::check_replica_limit(name, replicas)?;
			for container in self.replica_names_for(name, service, replicas) {
				if let Some(other) = owner.insert(container.clone(), name) {
					if other != name {
						return Err(ComposeError::Unsupported(format!(
							"services '{}' and '{}' both resolve to the container name '{}'; \
							 change the container_name of one of them",
							crate::error::sanitize_name(other),
							crate::error::sanitize_name(name),
							crate::error::sanitize_name(&container),
						)));
					}
				}
			}
		}
		Ok(())
	}
}

#[cfg(all(test, unix))]
#[path = "distinct_names_tests.rs"]
mod tests;
