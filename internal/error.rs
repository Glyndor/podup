//! Error types for the podup library.
//!
//! All fallible operations return [`Result<T>`], which is an alias for
//! `std::result::Result<T, ComposeError>`.

use std::fmt;

/// All errors produced by podup.
///
/// `#[non_exhaustive]`: new variants may be added in a minor release, so
/// downstream `match` arms must include a wildcard.
#[derive(Debug)]
#[non_exhaustive]
pub enum ComposeError {
	/// The compose YAML could not be deserialized.
	Parse(serde_yaml::Error),
	/// A referenced compose/include/extends file does not exist.
	FileNotFound(String),
	/// An underlying filesystem operation failed.
	Io(std::io::Error),
	/// The Podman libpod API returned an error or could not be reached.
	Podman(crate::libpod::PodmanError),
	/// A named service is not defined in the compose file.
	ServiceNotFound(String),
	/// `depends_on` forms a cycle, so no valid start order exists.
	CircularDependency(String),
	/// A service has neither an `image:` nor a `build:` section.
	NoImageOrBuild(String),
	/// A `${VAR}` with the `?err` modifier was required but unset.
	RequiredVarNotSet { var: String, msg: String },
	/// A `${…}` interpolation reference is malformed (e.g. an invalid character
	/// in the variable name, as in `${FOO BAR}` or `${FOO.BAR}`).
	InvalidSubstitution(String),
	/// A service did not become healthy within its dependency wait window.
	HealthCheckTimeout(String),
	/// A `ports:` entry could not be parsed.
	InvalidPort(String),
	/// Image build failed (context assembly or the Podman build step).
	Build(String),
	/// A `cp` (copy between a container and the host) operation failed — a missing
	/// destination directory, a non-directory path component, an unsupported
	/// endpoint, or a host-side packing/extraction error.
	Copy(String),
	/// `extends:` could not be resolved (missing file/service or a cycle).
	Extends(String),
	/// `include:` could not be resolved or merged.
	Include(String),
	/// The `watch` command failed (filesystem watch or sync action).
	Watch(String),
	/// A compose feature is recognized but unsupported on Podman/podup.
	Unsupported(String),
	/// A `run` container exited; carries its non-zero exit code so the CLI can
	/// propagate it as its own process exit status.
	RunExited(i64),
	/// `podup update` (self-update) failed.
	Update(String),
	/// An `external: true` secret/config/network/volume is absent.
	ExternalNotFound(String),
	/// A service is scaled to more than one replica but publishes a fixed host
	/// port, which only one container can bind.
	ScalePortConflict {
		service: String,
		replicas: usize,
		ports: Vec<u16>,
	},
	/// A container being waited on (`up`/`start --wait`, or a `service_healthy`
	/// dependency) exited non-zero before becoming ready.
	WaitServiceExited { container: String, code: i64 },
	/// A service requests more replicas than the configured ceiling, which would
	/// let an untrusted `deploy.replicas`/`scale:` drive unbounded container
	/// creation (host DoS).
	ReplicaLimitExceeded {
		service: String,
		replicas: usize,
		max: u32,
	},
	/// `start --wait --wait-timeout` elapsed before services became healthy.
	WaitTimeout { secs: u64 },
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
			Self::InvalidSubstitution(s) => {
				write!(f, "invalid variable substitution: {s}")
			}
			Self::HealthCheckTimeout(s) => write!(f, "health check timeout for container '{s}'"),
			Self::InvalidPort(s) => write!(f, "invalid port mapping: {s}"),
			Self::Build(s) => write!(f, "build error: {s}"),
			Self::Copy(s) => write!(f, "cp error: {s}"),
			Self::Extends(s) => write!(f, "extends error: {s}"),
			Self::Include(s) => write!(f, "include error: {s}"),
			Self::Watch(s) => write!(f, "watch error: {s}"),
			Self::Unsupported(s) => write!(f, "unsupported feature: {s}"),
			Self::RunExited(code) => write!(f, "run container exited with code {code}"),
			Self::Update(s) => write!(f, "update error: {s}"),
			Self::ExternalNotFound(s) => write!(f, "external resource not found: {s}"),
			Self::ScalePortConflict {
				service,
				replicas,
				ports,
			} => {
				let ports = ports
					.iter()
					.map(u16::to_string)
					.collect::<Vec<_>>()
					.join(", ");
				write!(
					f,
					"service '{service}' publishes fixed host port(s) [{ports}] but is scaled to \
					 {replicas} replicas; only one container can bind a host port. Use one of:\n  \
					 - remove the host port (e.g. `- \"80\"`) so Podman assigns a random one per \
					 replica\n  - put the service behind a reverse proxy and publish only the \
					 proxy's port\n  - reduce the service to a single replica"
				)
			}
			Self::WaitServiceExited { container, code } => write!(
				f,
				"container '{container}' exited with code {code} while waiting for it to be ready"
			),
			Self::ReplicaLimitExceeded {
				service,
				replicas,
				max,
			} => write!(
				f,
				"service '{service}' requests {replicas} replicas, which exceeds the limit of \
				 {max}; lower the count or raise the limit with PODUP_MAX_REPLICAS"
			),
			Self::WaitTimeout { secs } => write!(
				f,
				"timed out after {secs}s waiting for services to become healthy"
			),
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
			(
				"podman error:",
				ComposeError::Podman(crate::libpod::PodmanError::Api {
					status: 500,
					message: "boom".into(),
				}),
			),
			(
				"invalid variable substitution: bad",
				ComposeError::InvalidSubstitution("bad".into()),
			),
			("build error: b", ComposeError::Build("b".into())),
			("cp error: c", ComposeError::Copy("c".into())),
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
			(
				"service 'web' publishes fixed host port(s) [8080] but is scaled to 3 replicas",
				ComposeError::ScalePortConflict {
					service: "web".into(),
					replicas: 3,
					ports: vec![8080],
				},
			),
			(
				"container 'web' exited with code 7 while waiting for it to be ready",
				ComposeError::WaitServiceExited {
					container: "web".into(),
					code: 7,
				},
			),
			(
				"service 'web' requests 100000 replicas, which exceeds the limit of 256",
				ComposeError::ReplicaLimitExceeded {
					service: "web".into(),
					replicas: 100_000,
					max: 256,
				},
			),
			(
				"timed out after 30s waiting for services to become healthy",
				ComposeError::WaitTimeout { secs: 30 },
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
		// Parse and Podman also wrap a lower-level error and expose it.
		let parse =
			ComposeError::Parse(serde_yaml::from_str::<serde_yaml::Value>(":\0").unwrap_err());
		assert!(parse.source().is_some());
		let podman = ComposeError::Podman(crate::libpod::PodmanError::Api {
			status: 500,
			message: "boom".into(),
		});
		assert!(podman.source().is_some());
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
