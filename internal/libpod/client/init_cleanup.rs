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
		InitCleanup {
			remove: Some(move || {
				if let Ok(runtime) = tokio::runtime::Handle::try_current() {
					runtime.spawn(async move {
						if let Err(e) = Client::new(socket).delete_ok(&path).await {
							if !e.is_status(404) {
								tracing::warn!("pre_start cleanup: {e}");
							}
						}
					});
				}
			}),
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
