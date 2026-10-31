//! Directory permissions protect unit names from replacement by other users.

use std::io;
use std::path::Path;

pub(super) fn create(dir: &Path) -> io::Result<()> {
	let mut builder = std::fs::DirBuilder::new();
	builder.recursive(true);
	#[cfg(unix)]
	{
		use std::os::unix::fs::DirBuilderExt;
		builder.mode(0o700);
	}
	// DirBuilder leaves existing components untouched, including their modes.
	builder.create(dir)
}

pub(super) fn require_private(dir: &Path) -> io::Result<()> {
	create(dir)?;
	#[cfg(unix)]
	{
		use std::os::unix::fs::MetadataExt;
		let meta = std::fs::metadata(dir)?;
		if meta.uid() != effective_uid() {
			return Err(io::Error::new(
				io::ErrorKind::PermissionDenied,
				format!(
					"refusing unit directory {}: owned by UID {}, expected UID {}",
					dir.display(),
					meta.uid(),
					effective_uid()
				),
			));
		}
		// Others writing here can replace a unit, so that refuses. The group alone
		// is only a warning: Debian, Ubuntu and Fedora make home directories with
		// umask 002, so `~/.config/systemd/user` is 0775 for most users, and the
		// group is theirs alone.
		if meta.mode() & 0o002 != 0 {
			return Err(io::Error::new(
				io::ErrorKind::PermissionDenied,
				format!(
					"refusing unit directory {}: writable by others",
					dir.display()
				),
			));
		}
		if meta.mode() & 0o020 != 0 {
			let shown = crate::ui::sanitize_cell(&dir.display().to_string());
			eprintln!("podup: warning: unit directory {shown} is writable by its group; members of the group can replace the units");
		}
	}
	Ok(())
}

pub(super) fn warn_if_shared(dir: &Path) -> io::Result<()> {
	#[cfg(unix)]
	{
		use std::os::unix::fs::MetadataExt;
		if std::fs::metadata(dir)?.mode() & 0o022 != 0 {
			let shown = crate::ui::sanitize_cell(&dir.display().to_string());
			eprintln!("podup: warning: output directory {shown} is writable by group or others; unit files can be replaced by other users");
		}
	}
	#[cfg(not(unix))]
	let _ = dir;
	Ok(())
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn effective_uid() -> u32 {
	// SAFETY: geteuid takes no pointers and has no preconditions.
	unsafe { libc::geteuid() }
}
