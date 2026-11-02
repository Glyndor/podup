//! `ps --all-services`: the services the compose file declares, which needs the
//! file and nothing else. It is a method of the filter options and not of
//! `Engine`, so the command can run it without a Podman socket, like
//! `config --services`.

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

use super::filters::split_ps_filters;
use super::services::configured_services;
use super::PsFilterOptions;

impl PsFilterOptions {
	/// Print the configured service names, one per line, in file order,
	/// restricted by the positional `SERVICE` names.
	///
	/// The same checks as the container table run first: an unknown service and
	/// an unsupported `--filter` key are errors, because a dropped predicate
	/// answers a question the caller did not ask. The names are escaped like
	/// every other name podup prints, because the compose file's names are only
	/// checked by `config`, not by `ps`.
	pub fn print_configured_services(&self, file: &ComposeFile) -> Result<()> {
		for name in &self.services {
			if !file.services.contains_key(name) {
				return Err(ComposeError::ServiceNotFound(name.clone()));
			}
		}
		if let Some(u) = split_ps_filters(&self.filters).2.first() {
			return Err(ComposeError::Unsupported(format!(
				"unsupported ps filter {u:?}: expected name=<NAME> or status=<STATE>"
			)));
		}
		for name in configured_services(file, &self.services) {
			println!("{}", crate::ui::sanitize_cell(name));
		}
		Ok(())
	}
}
