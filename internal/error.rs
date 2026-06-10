//! Error types for the podup library.
//!
//! All fallible operations return [`Result<T>`], which is an alias for
//! `std::result::Result<T, ComposeError>`.

use std::fmt;

/// All errors produced by podup.
#[derive(Debug)]
pub enum ComposeError {
	Parse(serde_yaml::Error),
	FileNotFound(String),
	Io(std::io::Error),
	Podman(bollard::errors::Error),
	ServiceNotFound(String),
	CircularDependency(String),
	NoImageOrBuild(String),
	RequiredVarNotSet { var: String, msg: String },
	HealthCheckTimeout(String),
	InvalidPort(String),
	Build(String),
	Extends(String),
	Include(String),
	Watch(String),
	Unsupported(String),
	RunExited(i64),
}

impl fmt::Display for ComposeError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Parse(e) => write!(f, "failed to parse compose file: {e}"),
			Self::FileNotFound(s) => write!(f, "compose file not found: {s}"),
			Self::Io(e) => write!(f, "io error: {e}"),
			Self::Podman(e) => write!(f, "podman error: {e}"),
			Self::ServiceNotFound(s) => write!(f, "service '{s}' not found"),
			Self::CircularDependency(s) => write!(f, "circular dependency detected: {s}"),
			Self::NoImageOrBuild(s) => write!(f, "service '{s}' has no image or build config"),
			Self::RequiredVarNotSet { var, msg } => {
				write!(f, "required variable '{var}' is not set: {msg}")
			}
			Self::HealthCheckTimeout(s) => write!(f, "health check timeout for container '{s}'"),
			Self::InvalidPort(s) => write!(f, "invalid port mapping: {s}"),
			Self::Build(s) => write!(f, "build error: {s}"),
			Self::Extends(s) => write!(f, "extends error: {s}"),
			Self::Include(s) => write!(f, "include error: {s}"),
			Self::Watch(s) => write!(f, "watch error: {s}"),
			Self::Unsupported(s) => write!(f, "unsupported feature: {s}"),
			Self::RunExited(code) => write!(f, "run container exited with code {code}"),
		}
	}
}

impl std::error::Error for ComposeError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Parse(e) => Some(e),
			Self::Io(e) => Some(e),
			Self::Podman(e) => Some(e),
			_ => None,
		}
	}
}

impl From<serde_yaml::Error> for ComposeError {
	fn from(e: serde_yaml::Error) -> Self {
		Self::Parse(e)
	}
}

impl From<std::io::Error> for ComposeError {
	fn from(e: std::io::Error) -> Self {
		Self::Io(e)
	}
}

impl From<bollard::errors::Error> for ComposeError {
	fn from(e: bollard::errors::Error) -> Self {
		Self::Podman(e)
	}
}

pub type Result<T> = std::result::Result<T, ComposeError>;
