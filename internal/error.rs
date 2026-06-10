//! Error types for the podup library.
//!
//! All fallible operations return [`Result<T>`], which is an alias for
//! `std::result::Result<T, ComposeError>`.

use thiserror::Error;

/// All errors produced by podup.
#[derive(Debug, Error)]
pub enum ComposeError {
	#[error("failed to parse compose file: {0}")]
	Parse(#[from] serde_yaml::Error),

	#[error("compose file not found: {0}")]
	FileNotFound(String),

	#[error("io error: {0}")]
	Io(#[from] std::io::Error),

	#[error("podman error: {0}")]
	Podman(#[from] bollard::errors::Error),

	#[error("service '{0}' not found")]
	ServiceNotFound(String),

	#[error("circular dependency detected: {0}")]
	CircularDependency(String),

	#[error("service '{0}' has no image or build config")]
	NoImageOrBuild(String),

	#[error("required variable '{var}' is not set: {msg}")]
	RequiredVarNotSet { var: String, msg: String },

	#[error("health check timeout for container '{0}'")]
	HealthCheckTimeout(String),

	#[error("invalid port mapping: {0}")]
	InvalidPort(String),

	#[error("build error: {0}")]
	Build(String),

	#[error("extends error: {0}")]
	Extends(String),

	#[error("include error: {0}")]
	Include(String),

	#[error("watch error: {0}")]
	Watch(String),

	#[error("unsupported feature: {0}")]
	Unsupported(String),

	#[error("run container exited with code {0}")]
	RunExited(i64),
}

pub type Result<T> = std::result::Result<T, ComposeError>;
