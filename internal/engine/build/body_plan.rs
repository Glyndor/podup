//! Decide how the build context reaches libpod: a remote `remote=` URL
//! (no body), or a streamed context tar.
//!
//! Split out of `service.rs` so the dispatching loop there stays under
//! the source-line limit. The companion to
//! [`super::Engine::build_service`]: the body plan, the resolved
//! Dockerfile name and the in-tar build secret specs come back from
//! one call and feed straight into the `POST /libpod/build?` below.

use std::path::PathBuf;

use crate::compose::types::{BuildConfig, ComposeFile, Service};
use crate::error::{ComposeError, Result};

use super::context::INLINE_DOCKERFILE_NAME;
use super::stream::ContextSource;
use super::tags::is_remote_context;
use super::{BodyPlan, Engine};

/// What the request body carries, plus the companion values needed
/// to build it: the resolved Dockerfile name and the in-tar build
/// secret specs.
pub(in crate::engine) struct BuildPlan {
	/// Either `Empty` (remote context, no body) or `Stream { ... }`
	/// for the local-directory tar path.
	pub(in crate::engine) body: BodyPlan,
	/// Dockerfile name as the libpod endpoint should see it.
	pub(in crate::engine) dockerfile: String,
	/// In-tar secret specs the build endpoint should mount.
	pub(in crate::engine) secrets: Vec<String>,
}

/// Decide the build request body and the Dockerfile name.
///
/// Remote (`git://`/`https://`/`git@`) contexts are cloned server-side
/// by Podman via the `remote` query parameter, so there is no local
/// directory to tar. Tar-only features (inline Dockerfile, in-tar
/// build secrets) do not apply and are warned about. Local contexts
/// pre-validate the directory exists so a missing context surfaces
/// here, with the resolved path, rather than as a bare `io error`
/// from the tar walk; resolve `build.secrets` to in-tar files so the
/// libpod endpoint sees the form it expects.
#[allow(clippy::too_many_arguments)]
pub(in crate::engine) fn plan_build(
	engine: &Engine,
	service_name: &str,
	_service: &Service,
	file: &ComposeFile,
	build: &BuildConfig,
	context_str: &str,
) -> Result<BuildPlan> {
	if is_remote_context(context_str) {
		tracing::info!("building from remote context {context_str}");
		if build.dockerfile_inline().is_some() {
			tracing::warn!("build.dockerfile_inline is ignored for a remote build context");
		}
		if !build.secrets().is_empty() {
			tracing::warn!("build.secrets are ignored for a remote build context");
		}
		let dockerfile = build.dockerfile().unwrap_or("Dockerfile").to_string();
		return Ok(BuildPlan {
			body: BodyPlan::Empty,
			dockerfile,
			secrets: Vec::new(),
		});
	}
	let context_path: PathBuf = engine.base_dir.join(context_str);
	if let Err(e) = std::fs::metadata(&context_path) {
		return Err(ComposeError::BuildContext {
			service: service_name.to_string(),
			path: context_path.display().to_string(),
			source: e,
		});
	}
	tracing::info!("building from {}", context_path.display());

	let (secret_files, secrets) = engine.resolve_build_secrets(build, file)?;

	// The context tar is streamed to the socket (see the POST below),
	// never buffered, so a multi-gigabyte context doesn't inflate RSS.
	// Decide the source and the dockerfile name here; the blocking tar
	// walk happens while the request body is being sent.
	let (source, dockerfile) = match build.dockerfile_inline() {
		Some(inline) => (
			ContextSource::Inline(inline.to_string()),
			INLINE_DOCKERFILE_NAME.to_string(),
		),
		None => {
			let df = match build.dockerfile() {
				Some(name) => name.to_string(),
				None if !context_path.join("Dockerfile").is_file()
					&& context_path.join("Containerfile").is_file() =>
				{
					"Containerfile".to_string()
				}
				None => "Dockerfile".to_string(),
			};
			(ContextSource::Dockerfile(df.clone()), df)
		}
	};
	Ok(BuildPlan {
		body: BodyPlan::Stream {
			context: context_path,
			source,
			secrets: secret_files,
		},
		dockerfile,
		secrets,
	})
}
