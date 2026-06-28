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
	/// A replica index (`--index`) does not name a replica of the service (zero,
	/// or beyond the replica count). Kept distinct from [`Self::ServiceNotFound`]
	/// so the index hint renders outside the quoted service name.
	ReplicaIndex { service: String, index: u32 },
	/// A filesystem operation failed against a known path; carries the path so the
	/// message can name the offending file (Rust's `File::create`/`open` errors
	/// drop it).
	IoPath {
		path: String,
		source: std::io::Error,
	},
	/// A service's build context could not be accessed; names the service and the
	/// resolved context path instead of a bare `io error`.
	BuildContext {
		service: String,
		path: String,
		source: std::io::Error,
	},
	/// A `cp` operation failed (host-side packing, or a destination shape
	/// mismatch). Distinct from [`Self::Build`] so a copy never reads as a build
	/// error.
	Copy(String),
	/// A targeted service container is not running (e.g. `exec`/`attach` against a
	/// stopped or never-created container).
	NotRunning(String),
}

/// Cap on how much of a wrapped parse error is reflected back to the user.
/// serde_yaml embeds the offending scalar verbatim, so pointing `-f` at a
/// non-compose file would otherwise echo its entire contents (a host
/// file/secret disclosure); truncate it.
const MAX_PARSE_DETAIL: usize = 200;

/// Escape control characters (tabs, newlines, ESC, …) in an interpolated,
/// possibly-untrusted name before it reaches a terminal, so a crafted
/// service/container name cannot emit raw escape sequences. Printable characters
/// (including non-ASCII) pass through unchanged; only borrows when nothing needs
/// escaping.
fn sanitize_name(s: &str) -> std::borrow::Cow<'_, str> {
	if s.chars().any(char::is_control) {
		s.chars()
			.flat_map(|c| {
				if c.is_control() {
					c.escape_default().collect::<Vec<_>>()
				} else {
					vec![c]
				}
			})
			.collect::<String>()
			.into()
	} else {
		std::borrow::Cow::Borrowed(s)
	}
}

/// Truncate a wrapped lower-level error message to [`MAX_PARSE_DETAIL`] so an
/// embedded file scalar (potential secret) is not reflected untruncated.
fn truncate_detail(s: &str) -> String {
	if s.chars().count() <= MAX_PARSE_DETAIL {
		return s.to_string();
	}
	let cut: String = s.chars().take(MAX_PARSE_DETAIL).collect();
	format!("{cut}… (truncated)")
}

impl fmt::Display for ComposeError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Parse(e) => write!(
				f,
				"failed to parse compose file: {}",
				truncate_detail(&e.to_string())
			),
			Self::FileNotFound(s) => write!(f, "compose file not found: {}", sanitize_name(s)),
			Self::Io(e) => write!(f, "io error: {e}"),
			Self::Podman(e) => write!(f, "podman error: {e}"),
			Self::ServiceNotFound(s) => write!(f, "service '{}' not found", sanitize_name(s)),
			Self::CircularDependency(s) => write!(f, "{s}"),
			Self::NoImageOrBuild(s) => {
				write!(
					f,
					"service '{}' has no image or build config",
					sanitize_name(s)
				)
			}
			Self::RequiredVarNotSet { var, msg } => {
				write!(f, "required variable '{var}' is not set: {msg}")
			}
			Self::InvalidSubstitution(s) => {
				write!(f, "invalid variable substitution: {s}")
			}
			Self::HealthCheckTimeout(s) => {
				write!(
					f,
					"health check timeout for container '{}'",
					sanitize_name(s)
				)
			}
			Self::InvalidPort(s) => write!(f, "invalid port mapping: {s}"),
			Self::Build(s) => write!(f, "build error: {s}"),
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
				"container '{}' exited with code {code} while waiting for it to be ready",
				sanitize_name(container)
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
			Self::ReplicaIndex { service, index } => write!(
				f,
				"service '{}' has no replica {index} (replica indexes are 1-based)",
				sanitize_name(service)
			),
			Self::IoPath { path, source } => {
				write!(f, "io error: {}: {source}", sanitize_name(path))
			}
			Self::BuildContext {
				service,
				path,
				source,
			} => write!(
				f,
				"build context '{}' for service '{}': {source}",
				sanitize_name(path),
				sanitize_name(service)
			),
			Self::Copy(s) => write!(f, "cp error: {s}"),
			Self::NotRunning(s) => write!(f, "service '{}' is not running", sanitize_name(s)),
		}
	}
}

impl std::error::Error for ComposeError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			Self::Parse(e) => Some(e),
			Self::Io(e) => Some(e),
			Self::Podman(e) => Some(e),
			Self::IoPath { source, .. } | Self::BuildContext { source, .. } => Some(source),
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
			("c", ComposeError::CircularDependency("c".into())),
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
			(
				"service 'web' has no replica 99 (replica indexes are 1-based)",
				ComposeError::ReplicaIndex {
					service: "web".into(),
					index: 99,
				},
			),
			(
				"io error: /out/x.tar:",
				ComposeError::IoPath {
					path: "/out/x.tar".into(),
					source: std::io::Error::other("boom"),
				},
			),
			(
				"build context './ctx' for service 'web':",
				ComposeError::BuildContext {
					service: "web".into(),
					path: "./ctx".into(),
					source: std::io::Error::other("boom"),
				},
			),
			("cp error: oops", ComposeError::Copy("oops".into())),
			(
				"service 'web' is not running",
				ComposeError::NotRunning("web".into()),
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
	fn service_name_control_chars_are_escaped_in_display() {
		// A crafted name carrying an ESC sequence and newline must not reach the
		// terminal raw: the control bytes are escaped, the quotes preserved.
		let err = ComposeError::ServiceNotFound("we\x1b[31mb\n".into());
		let msg = err.to_string();
		assert!(!msg.contains('\x1b'), "ESC must be escaped: {msg:?}");
		assert!(!msg.contains('\n'), "newline must be escaped: {msg:?}");
		assert!(
			msg.contains("\\u{1b}") && msg.contains("\\n"),
			"got {msg:?}"
		);
	}

	#[test]
	fn parse_error_detail_is_truncated() {
		// serde_yaml embeds the offending scalar verbatim ("invalid type: string
		// \"…\""), so a huge value would otherwise be echoed in full — pointing
		// `-f` at a secret file would leak its contents. The Parse Display must cap
		// the reflected detail.
		let big = "x".repeat(5_000);
		let yaml = format!("\"{big}\"");
		let err = ComposeError::Parse(serde_yaml::from_str::<u8>(&yaml).unwrap_err());
		let msg = err.to_string();
		assert!(msg.starts_with("failed to parse compose file: "));
		assert!(
			msg.chars().count() < 400,
			"parse detail must be truncated, got {} chars",
			msg.chars().count()
		);
		assert!(msg.contains("truncated"));
	}

	#[test]
	fn replica_index_hint_is_outside_the_quoted_name() {
		// The hint must render after the closing quote, not inside the service name.
		let err = ComposeError::ReplicaIndex {
			service: "web".into(),
			index: 0,
		};
		let msg = err.to_string();
		assert!(msg.contains("'web'"), "service name stays clean: {msg:?}");
		assert!(!msg.contains("'web "), "hint leaked into the name: {msg:?}");
		assert!(msg.contains("1-based"));
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
