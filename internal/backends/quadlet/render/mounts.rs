use crate::compose::types::{VolumeMount, VolumeType};

use super::{unit_stem, ExternalNames};

/// Render a volume mount into a Quadlet `Volume=` value. A source naming a
/// declared volume gets a project-prefixed `.volume` suffix (so Quadlet wires it
/// to the generated unit, whose file name is also project-prefixed); an
/// external volume is named by the name it really has; an undeclared source
/// that names a host path is made absolute against the compose base directory;
/// any other source passes through verbatim. An empty source renders as just
/// the target. Long-form `read_only`, SELinux relabel (`z`/`Z`), bind
/// propagation, and `nocopy` opts are folded into the trailing `:opt,opt` field.
pub(in super::super) fn render_volume(
	vol: &VolumeMount,
	project: &str,
	declared_volumes: &[&str],
	external_volumes: &ExternalNames<'_>,
	base_dir: &std::path::Path,
) -> String {
	match vol {
		VolumeMount::Short(s) => {
			// A colon-less entry is a single in-container path (an anonymous
			// volume), not a `src:dst` pair, so there is no source to resolve.
			let Some((src, rest)) = s.split_once(':') else {
				return s.clone();
			};
			if declared_volumes.contains(&src) {
				format!("{}.volume:{}", unit_stem(project, src), rest)
			} else if let Some(name) = external_volumes.get(src) {
				format!("{name}:{rest}")
			} else {
				format!("{}:{}", resolve_source_path(src, base_dir), rest)
			}
		}
		VolumeMount::Long {
			source,
			target,
			read_only,
			bind,
			volume,
			..
		} => {
			let src = source.clone().unwrap_or_default();
			let src = if declared_volumes.contains(&src.as_str()) {
				format!("{}.volume", unit_stem(project, &src))
			} else if let Some(name) = external_volumes.get(src.as_str()) {
				name.to_string()
			} else {
				resolve_source_path(&src, base_dir)
			};
			// Collect mount options into the trailing `:opt,opt` field so nothing
			// is dropped: SELinux relabel (`z`/`Z`) and bind propagation are
			// security- and correctness-relevant on hardened hosts.
			let mut opts: Vec<String> = Vec::new();
			if *read_only == Some(true) {
				opts.push("ro".to_string());
			}
			if let Some(b) = bind {
				if let Some(selinux) = &b.selinux {
					opts.push(selinux.clone());
				}
				if let Some(propagation) = &b.propagation {
					opts.push(propagation.clone());
				}
			}
			if let Some(v) = volume {
				if v.nocopy == Some(true) {
					opts.push("nocopy".to_string());
				}
				// The mount-hardening trio (#1160). Dropping these here would
				// mean a compose file that hardens a mount exports a Quadlet
				// unit that silently does not.
				if v.noexec == Some(true) {
					opts.push("noexec".to_string());
				}
				if v.nosuid == Some(true) {
					opts.push("nosuid".to_string());
				}
				if v.nodev == Some(true) {
					opts.push("nodev".to_string());
				}
			}
			let mut out = if src.is_empty() {
				target.clone()
			} else {
				format!("{src}:{target}")
			};
			if !opts.is_empty() {
				out.push(':');
				out.push_str(&opts.join(","));
			}
			out
		}
	}
}

/// Resolve a volume source that names a host path into the absolute path the
/// unit has to carry.
///
/// systemd resolves a relative `Volume=` against the unit file's own
/// directory, which is not the compose file's, so a relative source would
/// mount a directory that does not exist once the unit is installed under
/// `~/.config/containers/systemd`. A `~` is not expanded by Quadlet either.
/// Both are resolved here, against the compose base directory, with the same
/// rule the live engine applies to a bind source, so the unit mounts the
/// directory `up` would mount.
///
/// An absolute source, and a bare name (a volume reference, not a path), are
/// returned unchanged.
fn resolve_source_path(src: &str, base_dir: &std::path::Path) -> String {
	if !(src.starts_with('.') || src.starts_with('~')) {
		return src.to_string();
	}
	crate::engine::container::resolve_bind_source(src, base_dir)
}

/// If `vol` is a long-form `type: tmpfs` mount, render it as a Quadlet `Tmpfs=`
/// value (`target[:size=…,mode=…]`); otherwise return `None` so the caller emits
/// a normal `Volume=`. A tmpfs mount written as `Volume=` would create a
/// persistent named/anonymous volume instead of an in-memory filesystem, a
/// silent semantic inversion. Size/mode mirror the runtime mount options.
pub(in super::super) fn render_tmpfs_mount(vol: &VolumeMount) -> Option<String> {
	let VolumeMount::Long {
		volume_type: VolumeType::Tmpfs,
		target,
		tmpfs,
		..
	} = vol
	else {
		return None;
	};
	let mut opts: Vec<String> = Vec::new();
	if let Some(t) = tmpfs {
		if let Some(size) = t.size {
			opts.push(format!("size={size}"));
		}
		if let Some(mode) = t.mode {
			opts.push(format!("mode={mode:o}"));
		}
	}
	if opts.is_empty() {
		Some(target.clone())
	} else {
		Some(format!("{target}:{}", opts.join(",")))
	}
}
