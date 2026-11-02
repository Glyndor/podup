use crate::error::{ComposeError, Result};

/// Combine the outcome of a one-off command with the outcome of its `--rm`
/// cleanup.
///
/// The cleanup is something the operator asked for explicitly, so a container
/// that could not be removed fails the run when the command itself succeeded:
/// the container is still there, with its writable layer and with whatever
/// the command wrote in it. When the command failed, its own error is the one
/// reported, and the incomplete cleanup is warned about next to it.
///
/// A container that is already gone is the idempotent success the flag
/// implies, so it is not a failure: `delete_ok` answers a 404 that way.
pub(super) fn finish_run(
	run_name: &str,
	outcome: Result<i64>,
	cleanup: std::result::Result<(), crate::libpod::PodmanError>,
) -> Result<()> {
	match (outcome, cleanup) {
		(Ok(0), Ok(())) => Ok(()),
		(Ok(0), Err(e)) => Err(ComposeError::Podman(e)),
		(Ok(code), Ok(())) => Err(ComposeError::RunExited(code)),
		(Ok(code), Err(e)) => {
			tracing::warn!("run {run_name}: the container could not be removed: {e}");
			Err(ComposeError::RunExited(code))
		}
		(Err(e), Ok(())) => Err(e),
		(Err(e), Err(cleanup_err)) => {
			tracing::warn!(
				"run {run_name}: the container could not be removed after the command failed: \
				 {cleanup_err}"
			);
			Err(e)
		}
	}
}

/// The `--rm` cleanup of a one-off container: what a removal that fails does
/// to the run's own result.
#[cfg(all(test, unix))]
mod cleanup_tests;
