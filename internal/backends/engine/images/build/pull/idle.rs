//! Inactivity budget for the finite image-pull response.

use std::time::Duration;

use crate::error::{ComposeError, Result};

pub(super) fn pull_idle_timeout() -> Result<Duration> {
	match std::env::var("PODUP_PULL_IDLE_TIMEOUT") {
		Ok(raw) => parse_idle_timeout(Some(&raw)),
		Err(std::env::VarError::NotPresent) => parse_idle_timeout(None),
		Err(std::env::VarError::NotUnicode(_)) => Err(invalid_timeout()),
	}
}

pub(super) fn parse_idle_timeout(raw: Option<&str>) -> Result<Duration> {
	let Some(raw) = raw else {
		return Ok(Duration::from_secs(300));
	};
	let seconds = raw
		.parse::<u64>()
		.ok()
		.filter(|seconds| *seconds > 0)
		.ok_or_else(invalid_timeout)?;
	Ok(Duration::from_secs(seconds))
}

fn invalid_timeout() -> ComposeError {
	ComposeError::Build(
		"PODUP_PULL_IDLE_TIMEOUT must be a positive integer number of seconds".to_string(),
	)
}
