//! The process the container runs: its entrypoint and its arguments.

use crate::compose::types::Service;

use super::{render_command, Section};

/// Emit the `Entrypoint=` and `Exec=` lines of the service.
pub(super) fn emit_command(service: &Service, container: &mut Section) {
	if let Some(entrypoint) = &service.entrypoint {
		container.add("Entrypoint", render_command(entrypoint));
	}
	if let Some(command) = &service.command {
		container.add("Exec", render_command(command));
	}
}
