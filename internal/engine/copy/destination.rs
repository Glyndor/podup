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

use crate::error::ComposeError;

/// Why `dst` must not be used as a `cp` destination, or `None` when every
/// component that exists is a real directory or file.
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
///
/// No `cfg` split: `symlink_metadata` exists on every target this builds
/// for, and on Windows `is_symlink` covers junctions as well as symbolic
/// links.
pub(super) fn destination_refusal(dst: &Path) -> Option<ComposeError> {
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
/// `/home` on some Linux systems are symlinks, and the way through is to
/// name the directory they point at.
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
