//! Join the context writer after its HTTP body has been dropped.

use std::time::Duration;

use tokio::task::JoinHandle;

use crate::error::{ComposeError, Result};

pub(super) async fn finish_upload(producer: Option<JoinHandle<Result<()>>>) -> Result<()> {
	let Some(mut producer) = producer else {
		return Ok(());
	};
	// The receiver must be dropped first to release a blocking_send. Bound
	// cleanup too: abort cannot interrupt filesystem IO in spawn_blocking.
	match tokio::time::timeout(Duration::from_secs(5), &mut producer).await {
		Ok(Ok(result)) => result,
		Ok(Err(_)) => Err(ComposeError::Build(
			"build-context producer failed".to_string(),
		)),
		Err(_) => {
			producer.abort();
			Err(ComposeError::Build(
				"timed out stopping the build-context producer".to_string(),
			))
		}
	}
}
