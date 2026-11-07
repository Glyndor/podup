//! Remove an init container even when dependency scheduling cancels its future.

use super::Client;

/// Preserve cleanup ownership while an init operation is suspended.
pub(crate) struct InitCleanup<F: FnOnce()> {
	remove: Option<F>,
}

impl<F: FnOnce()> InitCleanup<F> {
	/// Successful explicit removal must not schedule a second delete.
	pub(crate) fn disarm(&mut self) {
		self.remove = None;
	}
}

impl Client {
	/// Keep removal independent of the client borrowed by a canceled operation.
	pub(crate) fn init_cleanup(&self, path: String) -> InitCleanup<impl FnOnce()> {
		let socket = self.socket_path.clone();
		let pending = self.init_cleanups.clone();
		InitCleanup {
			remove: Some(move || {
				if let Ok(runtime) = tokio::runtime::Handle::try_current() {
					let removal = runtime.spawn(async move {
						if let Err(e) = Client::new(socket).delete_ok(&path).await {
							if !e.is_status(404) {
								tracing::warn!("pre_start cleanup: {e}");
							}
						}
					});
					// Kept, so the command that owns this client can wait for it:
					// a task nobody holds is lost when the process exits right after
					// the error that cancelled its init container.
					let mut pending = pending.lock().unwrap_or_else(|e| e.into_inner());
					pending.retain(|task| !task.is_finished());
					pending.push(removal);
				}
			}),
		}
	}

	/// Wait for the removals of init containers whose operation was cancelled.
	///
	/// Dropping a pending `pre_start` only schedules its removal, and a CLI that
	/// returns the error that caused the cancellation shuts its runtime down
	/// right after, so the removal has to be waited for before returning.
	pub(crate) async fn drain_init_cleanups(&self) {
		let tasks =
			std::mem::take(&mut *self.init_cleanups.lock().unwrap_or_else(|e| e.into_inner()));
		for task in tasks {
			let _ = task.await;
		}
	}
}

impl<F: FnOnce()> Drop for InitCleanup<F> {
	fn drop(&mut self) {
		if let Some(remove) = self.remove.take() {
			remove();
		}
	}
}

#[cfg(test)]
#[path = "init_cleanup_tests.rs"]
mod init_cleanup_tests;
