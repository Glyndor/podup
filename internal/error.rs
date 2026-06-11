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
	Podman(crate::libpod::PodmanError),
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
	Update(String),
	ExternalNotFound(String),
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
			Self::Update(s) => write!(f, "update error: {s}"),
			Self::ExternalNotFound(s) => write!(f, "external resource not found: {s}"),
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

impl From<crate::libpod::PodmanError> for ComposeError {
	fn from(e: crate::libpod::PodmanError) -> Self {
		Self::Podman(e)
	}
}

/// Convenience alias for `std::result::Result<T, ComposeError>`.
pub type Result<T> = std::result::Result<T, ComposeError>;

#[cfg(test)]
mod tests {
	use super::ComposeError;

	#[test]
	fn display_covers_all_variants() {
		let cases: &[(&str, ComposeError)] = &[
			(
				"failed to parse compose file:",
				ComposeError::Parse(serde_yaml::from_str::<serde_yaml::Value>(":\0").unwrap_err()),
			),
			(
				"compose file not found: f",
				ComposeError::FileNotFound("f".into()),
			),
			("io error:", ComposeError::Io(std::io::Error::other("x"))),
			(
				"service 's' not found",
				ComposeError::ServiceNotFound("s".into()),
			),
			(
				"circular dependency detected: c",
				ComposeError::CircularDependency("c".into()),
			),
			(
				"service 'svc' has no image or build config",
				ComposeError::NoImageOrBuild("svc".into()),
			),
			(
				"required variable 'V' is not set: reason",
				ComposeError::RequiredVarNotSet {
					var: "V".into(),
					msg: "reason".into(),
				},
			),
			(
				"health check timeout for container 'c'",
				ComposeError::HealthCheckTimeout("c".into()),
			),
			(
				"invalid port mapping: p",
				ComposeError::InvalidPort("p".into()),
			),
			("build error: b", ComposeError::Build("b".into())),
			("extends error: e", ComposeError::Extends("e".into())),
			("include error: i", ComposeError::Include("i".into())),
			("watch error: w", ComposeError::Watch("w".into())),
			(
				"unsupported feature: u",
				ComposeError::Unsupported("u".into()),
			),
			(
				"run container exited with code 1",
				ComposeError::RunExited(1),
			),
			("update error: u", ComposeError::Update("u".into())),
			(
				"external resource not found: external volume \"v\" does not exist",
				ComposeError::ExternalNotFound("external volume \"v\" does not exist".into()),
			),
		];
		for (expected_prefix, err) in cases {
			let msg = err.to_string();
			assert!(
				msg.starts_with(expected_prefix),
				"Display for {:?}: got {msg:?}, expected prefix {expected_prefix:?}",
				std::mem::discriminant(err),
			);
		}
	}

	#[test]
	fn source_provided_for_wrapped_variants() {
		use std::error::Error;
		let io = ComposeError::Io(std::io::Error::other("x"));
		assert!(io.source().is_some());
		let svc = ComposeError::ServiceNotFound("s".into());
		assert!(svc.source().is_none());
	}

	#[test]
	fn from_impls_convert_correctly() {
		let io_err = std::io::Error::other("x");
		let e: ComposeError = io_err.into();
		assert!(matches!(e, ComposeError::Io(_)));

		let yaml_err = serde_yaml::from_str::<serde_yaml::Value>(":\0").unwrap_err();
		let e: ComposeError = yaml_err.into();
		assert!(matches!(e, ComposeError::Parse(_)));

		let podman_err = crate::libpod::PodmanError::Api {
			status: 404,
			message: "not found".into(),
		};
		let e: ComposeError = podman_err.into();
		assert!(matches!(e, ComposeError::Podman(_)));
	}
}
