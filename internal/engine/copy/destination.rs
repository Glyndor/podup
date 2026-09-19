//! Host-side destination guard for container→host `cp`.
//!
//! The operator names a destination and the bytes must land at that name. A
//! symlink anywhere on the path breaks that: the kernel follows it, and
//! `tar`'s containment check canonicalises the extraction root, so the link
//! target becomes the accepted root. The link does not need a local attacker.
//! `extract_tar_guarded` creates the symlink entries an archive carries, so a
//! hostile container can plant `backup/data -> ~/.ssh` in one copy and wait
//! for the next `cp svc:/x ./backup/data/keys` to walk through it.
//!
//! #1736 refused a symlink at the last component only. `lstat` does not
//! follow the last component and follows every earlier one, so `real/link/out`
//! passed (#1764). A trailing separator defeated the last-component check as
//! well: `lstat("link/")` follows `link`.
//!
//! # When a root-level link is let through
//!
//! `/tmp` on macOS (`/tmp -> /private/tmp`) and `/home` on Fedora Atomic
//! (`/home -> var/home`) are root-owned symlinks directly in `/`. #1764
//! refused them too, so the operator had to spell `/private/tmp` or
//! `/var/home/me/out` instead, which is not what they typed. The guard still
//! refuses every other link; this is a single narrow exception that lets
//! through exactly one shape of root-level link and nothing else.
//!
//! The exception applies to a prefix `link` only when ALL of these hold:
//!
//! - `link` is the first component after the trusted root, that is, its
//!   parent IS the trusted root;
//! - `link` itself is owned by the trusted uid (`lstat` of the link, `uid()`);
//! - the trusted root itself is owned by the trusted uid AND has no write
//!   bit for group or other (`mode() & 0o022 == 0`).
//!
//! The trusted root on Unix is `/` and the trusted uid is `0`. On non-Unix
//! targets the exception does not exist: `trusted` is `None`, and every
//! symlink is refused as #1764 had it.
//!
//! After a trusted link the walk keeps going with the path AS WRITTEN
//! (`/tmp/out`); it does not substitute the link target: the kernel
//! resolves the trusted link, and every component below it is `lstat`ed
//! through it, so a link planted at `/private/tmp/x` is still seen as
//! `/tmp/x` and still refused.
//!
//! A trusted root link may also BE the destination itself, not only sit at a
//! prefix. The gates then see what it points at, not the link, so the
//! `destination_metadata` helper below follows the link in that one case and
//! leaves every other link reading as a link.
//!
//! # Why this is enough
//!
//! The hostile party of this module is the container. The container can
//! only put a link where the operator has already extracted into, which is
//! a directory the operator controls under `/`, not `/` itself. A link
//! directly in `/` means the operator already extracted into `/` as root,
//! at which point there is no host filesystem left to protect from this
//! guard. A link one level further down, owned by somebody else, or inside
//! a group-or-other-writable `/`, is still refused.
//!
//! The broader rule of thumb "trust any root-owned link in a root-owned
//! directory" is NOT what this exception does, on purpose: when podup runs
//! as root, links extracted from a hostile archive would be root-owned
//! inside root-owned destination directories, and the broader rule would
//! trust them. This exception is narrower (root-level only, no group or
//! other write on the root) so that case stays refused.
//!
//! # What this does not close
//!
//! This is a check on a path, and everything after it (`write`, `read_dir`,
//! `rename`, `set_permissions`, `tar`'s `unpack_in`) walks the path again. A
//! process that can write in one of the destination's ancestors can swap a
//! directory for a symlink between the two. Closing that needs a directory
//! handle opened component by component with `O_NOFOLLOW` and every later
//! operation made relative to it, and `tar::Entry::unpack_in` takes a path,
//! not a handle, so it means replacing the extractor.
//!
//! Who can win that race: a local process that already writes where the
//! operator is copying to, which has cheaper options there, and a container
//! that has a writable bind mount over one of the destination's ancestors
//! (`cp svc:/x ./data/out` with `./data` mounted into `svc`). The second is
//! the untrusted party of this module, so the race is a known limit and not
//! a closed one. A container without such a mount cannot touch the host
//! filesystem between the check and the use.
//!
//! Do not describe this guard as race-free, and do not replace it with a
//! canonicalised path passed downstream: a canonical string is re-walked
//! exactly like the original, and it hides the link that should have been
//! refused.

use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use crate::error::ComposeError;

/// Which root-level links the walk lets through, or `None` to refuse every
/// symlink (the non-Unix contract).
///
/// A trusted root is a directory the operator owns end-to-end, where a
/// root-level link is part of the system layout rather than something a
/// hostile container could have planted. On Unix the production value is
/// `root = "/", root_uid = 0, link_uid = 0`, which lets `/tmp` (macOS) and
/// `/home` (Fedora Atomic) through and refuses everything else.
///
/// `root_uid` and `link_uid` are kept as two separate expectations so each
/// owner check can be made to fail on its own in a test: a test process
/// cannot `chown`, so with one uid in the policy the two owner checks can
/// never be told apart.
#[cfg_attr(not(unix), allow(dead_code))]
pub(super) struct TrustedRoot<'a> {
	pub(super) root: &'a Path,
	/// Who must own the root directory.
	pub(super) root_uid: u32,
	/// Who must own a link for it to be let through.
	pub(super) link_uid: u32,
}

/// Why `dst` must not be used as a `cp` destination, or `None` when every
/// component that exists is a real directory or file. Production passes the
/// Unix trusted root (`/`, uids `0`) on Unix and `None` elsewhere; tests pass
/// a temp directory so the exception can be exercised without running as
/// root.
///
/// Walks `dst` one component at a time and `lstat`s each prefix, so no
/// symlink is followed while looking: a prefix is only extended after it was
/// seen to be a real directory. The prefix is rebuilt from components, which
/// drops a trailing separator, so the last component is never followed
/// either.
///
/// - Relative destinations are walked as written. The working directory is
///   held by the kernel as a directory, not as a path, so nothing above the
///   first written component is resolved again.
/// - `..` is not collapsed lexically (`link/..` is not `.`). It is left to
///   the kernel, which resolves it physically, and by then every component
///   before it was seen to be a real directory.
/// - The walk stops at the first component that does not exist, or that is
///   not a directory: nothing below it can exist, so nothing below it can be
///   a symlink. A destination whose parent is missing is reported by the
///   extraction itself, as before.
/// - A component that cannot be inspected for any other reason is refused.
///   Unknown must not become allowed.
/// - Root and prefix components (`/`, `C:\`, `\\?\`) are not inspected; they
///   cannot be symlinks.
/// - A symlink that satisfies the trusted-root check above is let through;
///   the walk continues with the path as written so every component below
///   the trusted link is still `lstat`ed through it.
///
/// No `cfg` split on the walk: `symlink_metadata` exists on every target
/// this builds for, and on Windows `is_symlink` covers junctions as well as
/// symbolic links. The trusted-root check is Unix-only and sits on its own
/// helper.
pub(super) fn destination_refusal(dst: &Path) -> Option<ComposeError> {
	destination_refusal_under(dst, unix_trusted_root())
}

/// The trusted-root policy production uses on this target. Unix: `/`, both
/// uids `0`. Anywhere else: `None`, the exception does not exist.
#[cfg(unix)]
fn unix_trusted_root() -> Option<TrustedRoot<'static>> {
	Some(TrustedRoot {
		root: Path::new("/"),
		root_uid: 0,
		link_uid: 0,
	})
}

#[cfg(not(unix))]
fn unix_trusted_root() -> Option<TrustedRoot<'static>> {
	None
}

/// The walk, with the trusted-root policy passed as data so the inner
/// function is exercisable without being root.
pub(super) fn destination_refusal_under(
	dst: &Path,
	trusted: Option<TrustedRoot<'_>>,
) -> Option<ComposeError> {
	#[cfg(not(unix))]
	let _ = &trusted;
	let mut current = PathBuf::new();
	let mut components = dst.components().peekable();
	while let Some(comp) = components.peek() {
		if matches!(comp, Component::Prefix(_) | Component::RootDir) {
			current.push(comp);
			components.next();
		} else {
			break;
		}
	}
	for comp in components {
		current.push(comp);
		match std::fs::symlink_metadata(&current) {
			Ok(meta) if meta.file_type().is_symlink() => {
				#[cfg(unix)]
				{
					if let Some(t) = &trusted {
						if trusted_root_link(&current, &meta, t) {
							continue;
						}
					}
				}
				return Some(symlink_refusal(dst, &current));
			}
			Ok(_) => {}
			Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
				return None;
			}
			Err(_) => {
				return Some(ComposeError::Copy(format!(
					"cp: refusing destination: cannot inspect {}",
					current.display()
				)));
			}
		}
	}
	None
}

/// Whether `link` is a root-level trusted link the walk should let through.
/// All three conditions of the documented rule must hold; failing any of
/// them falls through to the normal refusal.
#[cfg(unix)]
fn trusted_root_link(
	link: &Path,
	link_meta: &std::fs::Metadata,
	trusted: &TrustedRoot<'_>,
) -> bool {
	// The parent is compared against the trusted root as a directory, not as
	// a string, so the check does not depend on how the path is spelled.
	// `link/..`, `R/sub/../link`, and a relative path with 64 leading `..`s
	// all name the same link as `R/link`, and all three parents resolve to
	// the trusted root once canonicalised. An empty parent (e.g. `link/out`
	// from a working directory of `/`) is treated as `.`, which canonicalise
	// resolves against the working directory.
	//
	// Following links inside canonicalise is acceptable here: every
	// component before `link` was already walked and seen not to be a
	// symlink, since the only link the walk lets through is one directly in
	// the root. A parent that passes through such a link canonicalises to
	// the link's target, never to the root, so it is still refused.
	// `Path::new("tmp").parent()` is `Some("")`, not `None`, so both mean `.`.
	let parent = link
		.parent()
		.filter(|p| !p.as_os_str().is_empty())
		.unwrap_or_else(|| Path::new("."));
	let Ok(parent_canon) = std::fs::canonicalize(parent) else {
		return false;
	};
	let Ok(root_canon) = std::fs::canonicalize(trusted.root) else {
		return false;
	};
	if parent_canon != root_canon {
		return false;
	}
	link_meta.uid() == trusted.link_uid && root_is_trusted(trusted.root, trusted.root_uid)
}

/// Whether `root` is owned by `root_uid` and has no group or other write
/// bit. Returning `false` on `lstat` failure is the safe choice: missing or
/// unreadable root is not a trusted root.
#[cfg(unix)]
fn root_is_trusted(root: &Path, root_uid: u32) -> bool {
	let Ok(meta) = std::fs::symlink_metadata(root) else {
		return false;
	};
	meta.uid() == root_uid && meta.mode() & 0o022 == 0
}

/// Look at the destination the same way `symlink_metadata(dst)` did before
/// the trusted-root exception landed, but follow a trusted root link when
/// the walk would have let it through. The walk accepts `R/link/out`; this
/// function then sees `/tmp` (the directory behind `/tmp`) so the gates do
/// too, and `podup cp svc:/x /tmp` works the way `podup cp svc:/x /tmp/out`
/// does.
///
/// `dst` is `lstat`ed first; an untrusted link still reads as a link and the
/// gates still refuse it. When the result is a symlink AND the link
/// satisfies the very same three conditions the walk uses (via
/// [`trusted_root_link`], so the rule is not written twice), the function
/// calls `std::fs::metadata`, which follows the link, and returns the
/// directory (or file) behind it. On non-Unix the exception does not exist,
/// so the `lstat` result is always returned unchanged.
///
/// The destination path is rebuilt from `components()` before being
/// inspected, so a trailing separator is dropped and the kernel is never
/// asked to resolve the link through it.
///
/// A trusted link whose target does not exist returns `NotFound` from the
/// follow; both callers (`cp_destination_kind` and `extract_archive`) treat
/// any error here as "not an existing directory", which is what the
/// caller wants for that case.
pub(super) fn destination_metadata(dst: &Path) -> std::io::Result<std::fs::Metadata> {
	destination_metadata_under(dst, unix_trusted_root())
}

/// The metadata helper, with the trusted-root policy passed as data so the
/// inner function is exercisable without being root.
pub(super) fn destination_metadata_under(
	dst: &Path,
	trusted: Option<TrustedRoot<'_>>,
) -> std::io::Result<std::fs::Metadata> {
	#[cfg(not(unix))]
	let _ = &trusted;
	let rebuilt: PathBuf = dst.components().collect();
	let lstat = std::fs::symlink_metadata(&rebuilt)?;
	#[cfg(unix)]
	{
		if lstat.file_type().is_symlink() {
			if let Some(t) = &trusted {
				if trusted_root_link(&rebuilt, &lstat, t) {
					return std::fs::metadata(&rebuilt);
				}
			}
		}
	}
	Ok(lstat)
}

/// The refusal for a destination already classified as crossing a symlink.
///
/// Walks again to name the component. If the path changed in between and the
/// walk now comes back clean, the destination is still refused: the
/// classification that brought the caller here is not revisited.
pub(super) fn refusal_for(dst: &Path) -> ComposeError {
	destination_refusal(dst).unwrap_or_else(|| symlink_refusal(dst, dst))
}

/// Names the link when it is not the destination itself, so the operator can
/// see which part of the path to spell differently. `/tmp` on macOS and
/// `/home` on Fedora Atomic are root-owned symlinks directly in `/`; the
/// walk lets them through now, so only a link one level further down, in a
/// group-or-other-writable root, or owned by someone else still has to be
/// spelled as its target.
fn symlink_refusal(dst: &Path, link: &Path) -> ComposeError {
	if link == dst {
		return ComposeError::Copy(format!(
			"cp: refusing symlink destination: {}",
			dst.display()
		));
	}
	ComposeError::Copy(format!(
		"cp: refusing symlink destination: {} ({} is a symlink; name the directory it points at instead)",
		dst.display(),
		link.display()
	))
}
