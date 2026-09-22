//! `cp` command: copy files between a service container and the host.

use std::path::Path;

use bytes::Bytes;
use http_body_util::{BodyExt, Limited};

use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};
use crate::libpod::urlencoded;
use crate::libpod::API_PREFIX;

use super::Engine;
use verify::SentKind;

/// Crate-private so the fuzz harness behind the `test-helpers` feature can
/// reach `extract_tar_guarded` without widening the published API surface.
pub(crate) mod archive;
mod archive_pack;
mod destination;
mod pack;
pub(in crate::engine) mod pack_common;
mod progress;
mod stream;
mod upload;
pub(in crate::engine) mod verify;

/// Re-export the watch-sync packer at the engine level so the watch module
/// (`internal/engine/watch/mod.rs`) can reach the streaming upload shape
/// without widening the `pack` module's visibility to anything below
/// `pub(super)`.
pub(super) use pack::build_sync_tar_stream as build_sync_tar_stream_for_watch;

use archive::extract_archive;
pub(crate) use progress::ByteCounter as CpByteCounter;

/// Upper bound on a container→host `cp` archive buffered in memory. Without it a
/// hostile or huge container path would OOM the CLI. Generous (covers ordinary
/// file/dir copies); larger transfers should use `podman cp` directly.
const MAX_CP_ARCHIVE_BYTES: usize = 1024 * 1024 * 1024;

/// What `dst` looks like for the container→host cp routing decision.
///
/// Three cases drive [`Engine::cp_from_container`]:
/// - `Directory`: an existing real directory; the streaming extractor can
///   pipe the archive body straight into it without buffering.
/// - `Symlink`: a destination with a symlink at any component of its path,
///   the last one included, or with a component that could not be inspected
///   (refused the same way). `Path::is_dir` would follow the link and report
///   a directory, so the routing used to take the streaming branch and the
///   bytes landed in the link target rather than the named destination. The
///   same class of bug #1736 closed inside `extract_archive`; this is that
///   fix mirrored at the call site that picks between streaming and
///   buffering. #1736 looked at the last component only, and #1764 extended
///   it to the whole path.
/// - `NotADirectory`: a missing path (the buffered `extract_archive`
///   branch will create it) or an existing non-directory (the same
///   branch will land the single entry there).
pub(super) enum CpDestinationKind {
	Directory,
	Symlink,
	NotADirectory,
}

/// Classify `dst` for the cp routing. `symlink_metadata` is used rather
/// than `is_dir`/`exists` so a destination that is a symlink is reported
/// as a symlink, not the directory it points at. Without this, the
/// streaming branch would extract into the link target rather than the
/// named destination (#1736 + the call-site follow-up).
///
/// `symlink_metadata` alone only answers for the last component, so the
/// whole path goes through [`destination::destination_refusal`] first
/// (#1764). That module also records what the check does not close.
///
/// After the walk accepted the destination, the metadata is read through
/// [`destination::destination_metadata`] so a trusted root link whose
/// target IS the destination is followed (the walk would have let it
/// through); every other link still reads as a link.
pub(super) fn cp_destination_kind(dst: &Path) -> CpDestinationKind {
	if destination::destination_refusal(dst).is_some() {
		return CpDestinationKind::Symlink;
	}
	match destination::destination_metadata(dst) {
		Ok(meta) if meta.file_type().is_symlink() => CpDestinationKind::Symlink,
		Ok(meta) if meta.is_dir() => CpDestinationKind::Directory,
		_ => CpDestinationKind::NotADirectory,
	}
}

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

	/// 1-based replica index for a scaled service, `--index` (default: first).
	/// Builder-style.
	#[must_use]
	pub fn with_index(mut self, index: Option<u32>) -> Self {
		self.index = index;
		self
	}

	/// Follow symlinks in the host source before packing, `-L/--follow-link`.
	/// Builder-style.
	#[must_use]
	pub fn with_follow_link(mut self, follow_link: bool) -> Self {
		self.follow_link = follow_link;
		self
	}

	/// Archive mode, `-a/--archive`. Accepted for command-line compatibility:
	/// under rootless Podman the original uid/gid cannot be restored, and
	/// container→host extraction always applies podup's security-hardened mode
	/// sanitization, so this flag has no effect on the copied bytes.
	/// Builder-style.
	#[must_use]
	pub fn with_archive(mut self, archive: bool) -> Self {
		self.archive = archive;
		self
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

	async fn cp_from_container(
		&self,
		file: &ComposeFile,
		service_name: &str,
		container_path: &str,
		dst: &Path,
		opts: &CpOptions,
		progress: CpByteCounter,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self
			.live_replica_name_at(service_name, service, opts.index)
			.await?;

		let path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(container_path),
		);
		let resp = self
			.client
			.get_stream(&path)
			.await
			.map_err(ComposeError::Podman)?;
		let dst = dst.to_path_buf();

		// Two destination shapes, and only one of them can be streamed.
		//
		// An existing directory goes straight to `extract_tar_guarded`, which
		// walks the archive once, so the body can be piped into it and nothing
		// accumulates. That is also the shape that moves bulk data (`cp
		// svc:/var/lib/data ./backup/`), which is why it is the one worth
		// streaming.
		//
		// Any other destination goes through `extract_archive`, which reads the
		// archive twice: `archive_contains_dir` decides whether the destination
		// names a file or a directory to create, and only then does it extract.
		// A stream cannot be rewound, so that path still buffers. Making it
		// single-pass means changing what `cp` does with an ambiguous
		// destination, which is a behaviour decision rather than a memory one.
		//
		// The classification goes through `symlink_metadata` rather than
		// `is_dir` so a destination that is a symlink is reported as a symlink
		// rather than the directory it points at; the streaming branch would
		// otherwise extract into the link target. Same class of bug #1736
		// closed inside `extract_archive`, mirrored here at the routing site.
		match cp_destination_kind(&dst) {
			CpDestinationKind::Symlink => {
				return Err(destination::refusal_for(&dst));
			}
			CpDestinationKind::Directory => {
				return stream::extract_streamed(
					resp,
					dst,
					MAX_CP_ARCHIVE_BYTES as u64,
					Some(progress),
				)
				.await;
			}
			CpDestinationKind::NotADirectory => {
				// Fall through: `extract_archive` will either create a fresh
				// directory (directory source) or land the single entry at
				// exactly `dst` (single file source).
			}
		}

		// Cap the buffered archive so a huge/hostile container path cannot OOM
		// the CLI (the streaming `get_stream` path bypasses the client's own
		// cap). Kept as `Bytes` rather than `.to_vec()`: the extractor takes
		// `&[u8]`, which `Bytes` derefs to, and a `.to_vec()` here would hold a
		// second copy alive beside the first.
		let tar_bytes: Bytes = Limited::new(resp.into_body(), MAX_CP_ARCHIVE_BYTES)
			.collect()
			.await
			.map_err(|_| {
				ComposeError::Unsupported(format!(
					"cp: container archive exceeds {MAX_CP_ARCHIVE_BYTES} bytes; \
					 copy fewer files or use `podman cp` for very large transfers"
				))
			})?
			.to_bytes();
		progress.add(tar_bytes.len() as u64);

		tokio::task::spawn_blocking(move || extract_archive(&tar_bytes, &dst))
			.await
			.map_err(|e| ComposeError::Build(e.to_string()))??;

		Ok(())
	}

	/// Push a host file or directory into a service container.
	///
	/// # Concurrency contract: read before touching the two HEAD + PUT sequence
	///
	/// `cp_to_container` issues two `HEAD /archive` requests with the PUT
	/// between them, so a concurrent mutation in the window could land a
	/// successful-but-wrong-state PUT. The two callers in the codebase are
	/// the CLI `cp` subcommand and the `watch` sync path, both of which are
	/// called only while the per-project lock ([`crate::engine::lock`]) is
	/// held by the mutating stage: `lock_project` serialises a single
	/// `podup` process against any other `podup` process working on the same
	/// project, closing the **cross-invocation** case.
	///
	/// The **within-invocation** case, a foreign actor (a manual
	/// `podman exec`, another compose stack on the same machine, the user
	/// running `podman cp` in another shell) mutating the destination
	/// between the two HEADs, is closed by libpod itself: the archive PUT
	/// extracts into a directory that we have just confirmed exists and is
	/// a directory, so a foreign `rm -rf` racing in is rejected by the
	/// second PUT, not silently succeeded. The `extract_stat_path` HEAD
	/// below is what makes that property hold; do not skip it.
	#[allow(clippy::too_many_arguments)]
	async fn cp_to_container(
		&self,
		file: &ComposeFile,
		service_name: &str,
		src: &Path,
		container_path: &str,
		opts: &CpOptions,
		contents: bool,
		progress: CpByteCounter,
	) -> Result<()> {
		let service = file
			.services
			.get(service_name)
			.ok_or_else(|| ComposeError::ServiceNotFound(service_name.into()))?;
		let container_name = self
			.live_replica_name_at(service_name, service, opts.index)
			.await?;

		// Match `docker cp` destination semantics. The libpod archive PUT extracts
		// the tar *at* a directory, so:
		//  - dest is an existing directory (or ends in `/`)  → copy the source in
		//    under its own name (PUT to the dest dir);
		//  - dest is anything else (a new name, or a file)   → rename the source to
		//    the dest's basename and PUT to the dest's parent.
		// Without this, `cp file svc:/path/newname` created `newname/` as a
		// directory holding the source instead of a file named `newname`.
		//
		// The `contents` branch (trailing `/.` on the host source) short-
		// circuits the rename: the archive holds no wrapper, and the PUT
		// destination IS the final location (whether it already exists or not).
		// That matches `podman cp host/. svc:/path/newname`, which lands the
		// contents directly at `/path/newname/`.
		let stat_path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(container_path),
		);
		let dest_is_dir = self.client.head_path_is_dir(&stat_path).await? == Some(true);

		let (extract_dir, rename) = if contents {
			// Contents land at the destination itself; libpod creates it if
			// it does not exist. No rename, no wrapper.
			(container_path.trim_end_matches('/').to_string(), None)
		} else if dest_is_dir || container_path.ends_with('/') {
			(container_path.trim_end_matches('/').to_string(), None)
		} else {
			let trimmed = container_path.trim_end_matches('/');
			let (parent, name) = trimmed.rsplit_once('/').unwrap_or(("", trimmed));
			let parent = if parent.is_empty() { "/" } else { parent };
			(parent.to_string(), Some(name.to_string()))
		};

		// Validate the extraction directory exists and is itself a directory before
		// PUTting the archive. Without this, libpod silently auto-creates a missing
		// parent chain (diverging from docker/podman `cp`, which error with "no such
		// directory"); and when a path component is a regular file the archive PUT
		// never gets a response, blocking the full READ_TIMEOUT window instead of
		// failing fast.
		let extract_stat_path = format!(
			"{API_PREFIX}/containers/{}/archive?path={}",
			urlencoded(&container_name),
			urlencoded(&extract_dir),
		);
		match self.client.head_path_is_dir(&extract_stat_path).await? {
			Some(true) => {}
			Some(false) => {
				return Err(ComposeError::Copy(format!(
					"cp: not a directory: {extract_dir}"
				)));
			}
			None => {
				return Err(ComposeError::Copy(format!(
					"cp: no such directory: {extract_dir}"
				)));
			}
		}

		let src_buf = src.to_path_buf();
		let follow = opts.follow_link;
		let rename_for_pack = rename.clone();
		// Stream the tar bytes into a bounded channel rather than building
		// the whole archive into a `Vec<u8>` first (#1844). The recorded
		// entry list is what the post-PUT confirmation compares against
		// once the body bytes are gone.
		let packed = pack::pack_path_stream(
			&src_buf,
			follow,
			rename_for_pack.as_deref(),
			contents,
			progress.inner().clone(),
		);

		// Contents-packed archives have no wrapper entry: `tree_landed` walks
		// the recorded list and asks about each entry against the destination.
		// The existing `entry`-based confirmation is only meaningful when the
		// archive is wrapped under a single name, which `contents=true`
		// removes.
		let entry = if contents {
			String::new()
		} else {
			rename.clone().unwrap_or_else(|| {
				src.file_name()
					.map(|n| n.to_string_lossy().into_owned())
					.unwrap_or_default()
			})
		};
		let uploaded_kind = uploaded_entry_kind(src, follow);
		self.put_archive_verified(&container_name, &extract_dir, &entry, packed, uploaded_kind)
			.await
	}
}

/// Pick the row name for the live board from the `cp` endpoint pair.
///
/// Always the destination side, since the destination is the side the
/// operator was thinking about when they typed the command: the local
/// path for container->host (`svc:/path -> /host/dst`), the service ref
/// for host->container (`/host/src -> svc:/path`). One branch decides,
/// because the endpoint parser has already rejected `SERVICE:PATH` on
/// both sides and `-` on either side.
fn cp_row_name(_src: &str, dst: &str) -> String {
	dst.to_string()
}

/// What the destination entry must look like for a single-entry upload to have
/// landed, or `None` when there is nothing comparable.
///
/// A regular file source expects `SentKind::File(size)`, where the size is the
/// file's actual length on disk. The kind is part of the comparison (an empty
/// file over an unchanged zero-length FIFO would otherwise pass on size
/// alone). A symlink at the source without `-L/--follow-link` expects
/// `SentKind::Link(target)`, where `target` is what the tar will carry as
/// the link name (read out of the host symlink with `fs::read_link`). The
/// destination is asked about that target after Podman normalises it the same
/// way it normalises `linkTarget`, and the byte length of the target is
/// checked against `stat.size` independently. A directory source returns `None`
/// because its own size says nothing about its children; that case is
/// confirmed entry by entry (`verify::tree_landed`), never on this. Anything
/// else stays unverifiable and fails closed rather than confirming on the
/// wrong kind.
///
/// Extracted so it is reachable from a test. Inside the async upload it was
/// covered only by running against a real container, and a mutation
/// replacing the regular-file length with a constant survived the whole
/// unit suite.
pub(super) fn uploaded_entry_kind(src: &std::path::Path, follow_link: bool) -> Option<SentKind> {
	let meta = if follow_link {
		std::fs::metadata(src).ok()?
	} else {
		std::fs::symlink_metadata(src).ok()?
	};
	let kind = meta.file_type();
	if kind.is_symlink() && !follow_link {
		// `read_link` returns the link target as the host filesystem stores
		// it; that is the same bytes `pack_path` will put in the tar's
		// linkname field, so it is the value the destination's stat will
		// be asked to match. A path that is not valid UTF-8 makes the
		// entry unverifiable on the host side (we cannot normalise what
		// we cannot name), and the archive PUT would refuse it anyway.
		let target = std::fs::read_link(src).ok()?;
		let target = target.to_str().map(str::to_string)?;
		Some(SentKind::Link(target))
	} else if kind.is_file() {
		Some(SentKind::File(meta.len()))
	} else {
		None
	}
}

/// Join a container directory and an entry name into one path, without doubling
/// the separator when the directory already ends in `/` (so root `/` yields
/// `/name`, not `//name`). Pure so the join is unit-tested without a container.
fn join_archive_path(dir: &str, entry: &str) -> String {
	if dir.ends_with('/') {
		format!("{dir}{entry}")
	} else {
		format!("{dir}/{entry}")
	}
}

/// Whether the source string was written as the contents cue: a trailing
/// `/.` (or just `.`) means "copy the directory's contents, not the directory
/// itself". Detected on the original string because `Path::new("payload/.")`
/// has `file_name() == Some("payload")`: by the time the path reaches the
/// packer the cue is gone. The cue is present when, after stripping any
/// trailing characters for which `std::path::is_separator` is true, the
/// source is exactly `.` or ends with a separator followed by `.`. On Unix a
/// backslash is an ordinary filename character, so the cue only fires on
/// the forward-slash shape there; on Windows it fires on both the
/// forward-slash and the backslash shape. `..` and a path whose last
/// component is `..` are not the cue, and an empty source is not the cue.
fn has_dot_contents_suffix(src: &str) -> bool {
	let trimmed = src.trim_end_matches(std::path::is_separator);
	trimmed == "."
		|| trimmed
			.strip_suffix('.')
			.is_some_and(|rest| rest.ends_with(std::path::is_separator))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Reject the `cp` endpoint forms podup explicitly does not support, with a
/// clear diagnostic rather than letting them fall through to a local path that
/// happens to be named `-` or `SERVICE:`.
///
/// - `-` (stdin/stdout streaming) is not implemented.
/// - `SERVICE:` (a colon with an empty container path) is a malformed reference.
///
/// A plain local path (no colon, or a colon that is part of an ordinary host
/// path / Windows drive) is left to [`parse_endpoint`].
fn check_endpoint(s: &str) -> Result<()> {
	if s == "-" {
		return Err(ComposeError::Unsupported(
			"cp: stdin/stdout ('-') is not supported".into(),
		));
	}
	if let Some((svc, path)) = s.split_once(':') {
		// A Windows drive letter (`C:\...`) is a local path, not a service ref.
		#[cfg(windows)]
		if svc.len() == 1 && svc.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
			return Ok(());
		}
		if !svc.is_empty() && path.is_empty() {
			return Err(ComposeError::Copy(format!(
				"cp: empty container path in '{s}' (expected SERVICE:PATH)"
			)));
		}
	}
	Ok(())
}

fn parse_endpoint(s: &str) -> Option<(&str, &str)> {
	if s == "-" {
		return None;
	}
	// `SERVICE:PATH`: colon must not be the first character and path cannot be empty.
	let (svc, path) = s.split_once(':')?;
	if svc.is_empty() || path.is_empty() {
		return None;
	}
	// On Windows, an absolute path like `C:\path` has a single-char drive prefix,
	// treat those as local paths, not service endpoints. This must NOT apply on
	// Unix, where a one-character service name (`c:/path`) is perfectly valid and
	// would otherwise be rejected as a bogus "drive".
	#[cfg(windows)]
	if svc.len() == 1 && svc.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
		return None;
	}
	Some((svc, path))
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "copy_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "copy_upload_tests.rs"]
mod upload_tests;

#[cfg(test)]
#[path = "copy/destination_tests.rs"]
mod destination_tests;

#[cfg(test)]
#[path = "copy/destination_trusted_tests.rs"]
mod destination_trusted_tests;

#[cfg(all(test, unix))]
#[path = "copy/cp_progress_tests.rs"]
mod cp_progress_tests;
