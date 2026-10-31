//! Who owns a Podman object a project is about to reuse or remove.
//!
//! Containers, pods, networks and volumes are addressed by name, and a name in
//! a compose file can coincide with an object podup did not create. The
//! `podup.project` label is the only ownership evidence podup writes, so every
//! path that reuses or removes an object by name asks here first.

use crate::engine::Engine;
use crate::error::{ComposeError, Result};
use crate::libpod::API_PREFIX;

/// What the host says about the owner of a named object.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::backends::engine) enum Owner {
	/// The object does not exist.
	Absent,
	/// The object exists and carries no `podup.project` label.
	Unlabelled,
	/// The object carries `podup.project=<project>`.
	Project(String),
}

impl Owner {
	/// Read the owner out of an inspect document. `label_pointers` lists the
	/// JSON pointers where the label map can sit, because libpod spells it
	/// `Labels`, `labels` or `Config.Labels` depending on the object.
	fn from_inspect(value: &serde_json::Value, label_pointers: &[&str]) -> Self {
		label_pointers
			.iter()
			.filter_map(|p| value.pointer(p))
			.find_map(|labels| labels.get("podup.project").and_then(|v| v.as_str()))
			.map_or(Self::Unlabelled, |project| {
				Self::Project(project.to_string())
			})
	}
}

impl Engine {
	/// Inspect `<api_segment>/<name>/json` and report who owns it. A 404 is
	/// `Owner::Absent`; any other failure propagates, so a daemon that is
	/// down is never read as "nothing there".
	pub(in crate::backends::engine) async fn object_owner(
		&self,
		api_segment: &str,
		name: &str,
		label_pointers: &[&str],
	) -> Result<Owner> {
		let path = format!(
			"{API_PREFIX}/{api_segment}/{}/json",
			crate::libpod::urlencoded(name)
		);
		match self.client.get_json::<serde_json::Value>(&path).await {
			Ok(value) => Ok(Owner::from_inspect(&value, label_pointers)),
			Err(e) if e.is_status(404) => Ok(Owner::Absent),
			Err(e) => Err(ComposeError::Podman(e)),
		}
	}

	/// Refuse to act on `name` unless it is absent or this project's. The
	/// error names the object and says how to share it on purpose.
	pub(in crate::backends::engine) fn require_owned(
		&self,
		kind: &str,
		name: &str,
		owner: &Owner,
	) -> Result<()> {
		match owner {
			Owner::Absent => Ok(()),
			Owner::Project(p) if *p == self.project => Ok(()),
			Owner::Project(p) => Err(ComposeError::Unsupported(format!(
				"{kind} '{name}' is labelled podup.project={p}; refusing to touch it from \
		         this project ('{}'). Declare it with 'external: true' to share it.",
				self.project
			))),
			Owner::Unlabelled => Err(ComposeError::Unsupported(format!(
				"{kind} '{name}' carries no podup.project label; refusing to touch it from \
		         this project ('{}') without ownership evidence. Declare it with \
		         'external: true' to share it.",
				self.project
			))),
		}
	}
}
