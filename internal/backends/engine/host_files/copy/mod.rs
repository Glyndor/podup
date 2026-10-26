//! `cp` command: copy files between a service container and the host.

use std::path::Path;

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

use crate::engine::Engine;

mod endpoint;
mod extract;
mod from_container;
mod pack;
mod progress;
mod to_container;
pub(in crate::backends::engine) mod upload;

/// Crate-private so the fuzz harness behind the `test-helpers` feature can
/// reach `extract_tar_guarded` without widening the published API surface.
#[cfg(feature = "test-helpers")]
pub(crate) use extract::archive;

/// Re-export the watch-sync packer at the engine level so the watch module
/// (`internal/backends/engine/host_files/watch/mod.rs`) can reach the streaming upload shape
/// without widening the transport module's visibility to anything below
/// `pub(in crate::backends::engine::host_files::copy)`.
pub(in crate::backends::engine) use pack::transport::build_sync_tar_stream as build_sync_tar_stream_for_watch;

/// The shared walk-and-record helpers, under the name the watch-sync packer
/// (`internal/backends/engine/host_files/watch/sync.rs`) imports them by.
pub(in crate::backends::engine) use pack::common as pack_common;

/// The confirmation side of the upload, under the name the watch-sync packer
/// imports the recorder by.
pub(in crate::backends::engine) use upload::verify;

pub(crate) use progress::ByteCounter as CpByteCounter;

use endpoint::{check_endpoint, cp_row_name, has_dot_contents_suffix, parse_endpoint};
// The classifier and its verdict enum moved to the direction that asks the
// question. The copy tests reach them through this name, the same path they
// took while both lived here; nothing outside a test build names either, so
// the binding is only worth having under `cfg(test)`.
#[cfg(test)]
use from_container::{cp_destination_kind, CpDestinationKind};
pub(in crate::backends::engine) use to_container::uploaded_entry_kind;
// `verify` and `upload` reach the re-verify stat through this name, the same
// path they took while the join lived in this file.
use to_container::join_archive_path;

/// Options for [`Engine::cp_with_options`], mirroring `docker compose cp` flags.
///
/// `#[non_exhaustive]` since 4.0.0, so a new field can be added in a minor
/// release without breaking every external caller that built the struct with
/// a literal. Construct it via [`CpOptions::new`] or the `with_*` builders
/// below; a struct literal is refused outside this crate, which is what buys
/// the room to grow.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CpOptions {
	/// 1-based replica index for a scaled service, `--index` (default: first).
	pub index: Option<u32>,
	/// Follow symlinks in the host source before packing, `-L/--follow-link`.
	pub follow_link: bool,
	/// Archive mode, `-a/--archive`. Accepted for command-line compatibility:
	/// under rootless Podman the original uid/gid cannot be restored, and
	/// container→host extraction always applies podup's security-hardened mode
	/// sanitization, so this flag has no effect on the copied bytes.
	pub archive: bool,
}

impl CpOptions {
	/// Every `docker compose cp` flag, in CLI order. A constructor rather than
	/// a struct literal because the type is `#[non_exhaustive]`, so the next
	/// flag to land is not a breaking change for anyone building one.
	pub fn new(index: Option<u32>, follow_link: bool, archive: bool) -> Self {
		Self {
			index,
			follow_link,
			archive,
		}
	}
}

impl Engine {
	/// Copy between a service container and the local filesystem.
	///
	/// Either `src` or `dst` (but not both) must have the form `SERVICE:PATH`.
	/// The other side is a local path. `SERVICE:-` / `-:SERVICE` for stdin/stdout
	/// is not supported.
	pub async fn cp(&self, file: &ComposeFile, src: &str, dst: &str) -> Result<()> {
		self.cp_with_options(file, src, dst, CpOptions::default())
			.await
	}

	/// Copy with `docker compose cp` options: `--index` (target a specific
	/// replica), `-L/--follow-link` (follow host symlinks when uploading) and
	/// `-a/--archive` (accepted for compatibility; see [`CpOptions::archive`]).
	pub async fn cp_with_options(
		&self,
		file: &ComposeFile,
		src: &str,
		dst: &str,
		opts: CpOptions,
	) -> Result<()> {
		// Reject the explicitly-unsupported endpoint forms (`-` for stdin/stdout,
		// and a `SERVICE:` with an empty container path) with a clear message
		// before they silently fall through to a local file literally named `-`
		// or `SERVICE:`.
		check_endpoint(src)?;
		check_endpoint(dst)?;
		// The row name on the live board. Always the destination side, since
		// the destination is the side the operator was thinking about when they
		// typed the command: the local path for container->host (`svc:/path ->
		// /host/dst`), the service ref for host->container (`/host/src ->
		// svc:/path`).
		let row_name = cp_row_name(src, dst);
		let counter = CpByteCounter::new();
		let emitter = progress::spawn_emitter("Cp", row_name.clone(), counter.clone());
		crate::ui::progress::begin(vec![(crate::ui::progress::Kind::Cp, row_name.clone())]);
		crate::ui::progress::start("Cp", &row_name, "Copying");
		let result = match (parse_endpoint(src), parse_endpoint(dst)) {
			(Some((service, container_path)), None) => {
				self.cp_from_container(
					file,
					service,
					container_path,
					Path::new(dst),
					&opts,
					counter.clone(),
				)
				.await
			}
			(None, Some((service, container_path))) => {
				// `cp host/. svc:/X` copies the host directory's *contents*
				// into the archive, the way `docker cp` / `podman cp` treat a
				// trailing `/.`. The dot must be detected on the original
				// string, before `Path::new(src)` drops it; threading that fact
				// down is what makes the packer behave differently.
				let contents = has_dot_contents_suffix(src);
				self.cp_to_container(
					file,
					service,
					Path::new(src),
					container_path,
					&opts,
					contents,
					counter.clone(),
				)
				.await
			}
			(Some(_), Some(_)) => Err(ComposeError::Unsupported(
				"cp: both src and dst cannot be SERVICE:PATH".into(),
			)),
			(None, None) => Err(ComposeError::Unsupported(
				"cp: one of src or dst must be SERVICE:PATH".into(),
			)),
		};
		emitter.stop();
		let bytes = counter.load();
		let verb = if result.is_ok() {
			progress::format_copied_verb(bytes)
		} else {
			"Failed".to_string()
		};
		crate::ui::progress_line("Cp", &row_name, &verb);
		crate::ui::progress::end();
		result
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod cp_progress_tests;
