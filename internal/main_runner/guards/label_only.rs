//! "Missing compose file is fine" guard for the label-only commands.
//!
//! `ps` and `events` answer purely by the `podup.project` label and may
//! succeed without a compose file. A file the operator named via `-f` or
//! `COMPOSE_FILE` and that is not there is a different case: answering as if
//! none were given turns a typo into an empty listing and exit 0 (#1687).

use std::path::PathBuf;

use crate::startup::resolve::compose_files_were_named;

/// The label-only commands (`ps`, `events`) run without a compose file so a
/// directory that has none still answers by project label. A file the
/// operator named and that is not there is a different case: answering as if
/// none were given turns a typo into an empty listing and exit 0 (#1687).
pub(crate) fn fail_on_named_file_missing(
	explicit: &[PathBuf],
	compose_files: &[PathBuf],
) -> Result<(), podup::ComposeError> {
	if compose_files_were_named(explicit) && !compose_files.iter().any(|p| p.is_file()) {
		let named = compose_files
			.first()
			.map(|p| p.display().to_string())
			.unwrap_or_default();
		return Err(podup::ComposeError::FileNotFound(named));
	}
	Ok(())
}
