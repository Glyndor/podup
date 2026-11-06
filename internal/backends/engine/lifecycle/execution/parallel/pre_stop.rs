//! The `pre_stop` hooks of a service, run in a replica before it is stopped.

use crate::compose::types::LifecycleHook;
use crate::engine::Engine;

impl Engine {
	/// Run `hooks` inside `target`, in order, before the stop or restart signal.
	///
	/// Best effort, as for `down`: a hook that fails, or one that cannot run
	/// because the container is not running, is logged at debug level and the
	/// stop goes ahead. `target` is the ID of the snapshot when there is one, so
	/// the hook cannot reach a new container that reused the name.
	pub(in crate::backends::engine::lifecycle) async fn run_pre_stop(
		&self,
		name: &str,
		target: &str,
		hooks: &[LifecycleHook],
	) {
		for hook in hooks {
			if let Err(e) = self.run_lifecycle_hook(target, hook).await {
				tracing::debug!("pre_stop hook {name}: {e}");
			}
		}
	}
}
