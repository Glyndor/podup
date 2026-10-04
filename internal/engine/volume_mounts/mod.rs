//! Volume mount helpers.
//!
//! [`build_mounts_all`] converts all `volumes:` entries into OCI `Mount` entries
//! and `NamedVolume` entries for the SpecGenerator. Named volumes go in
//! `volumes`; everything else (bind, tmpfs, npipe, cluster) goes in `mounts`.
//! Secrets and configs are not here: every source is a Podman-native secret
//! attached to the spec, never a mount.

use std::path::Path;

use crate::compose::types::{Service, VolumeMount, VolumeType};
use crate::libpod::types::container::{Mount, NamedVolume};

mod spec;
use spec::{access_opts, extend_bind_opts_str, extend_volume_opts_str, parse_volume_string};

/// What `ensure_bind_source` decided at the host-source path.
///
/// The defect this guards against printed a warning without changing
/// the filesystem: `create_dir_all` on an existing file returns `EEXIST`
/// and the helper only logged it. A test that only inspects the
/// filesystem afterwards cannot see whether the helper decided
/// correctly, so the decision itself is the thing a test can read and
/// assert.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum BindSource {
	/// Something already exists at the path; nothing was attempted.
	Present,
	/// Nothing existed; the directory was created.
	Created,
	/// Nothing existed and creating it failed, or the path could not be
	/// inspected. The message is what the warning prints.
	Failed(String),
}

/// Ensure the host source path for a bind mount exists before podman mounts
/// it. `symlink_metadata` returns `Ok` for any existing entry (file,
/// directory, symlink, including a dangling symlink), so a pre-existing source
/// is left alone: only a missing path leads to `create_dir_all`. A real
/// failure to create the directory returns `Failed` with today's warning
/// text; a non-`NotFound` stat error (e.g. permission denied on the
/// parent) returns `Failed` with the same text so the operator still sees
/// why the mount may fail. `create_dir_all` on a source that already exists
/// would return `EEXIST` and print `create_host_path: failed to create …
/// File exists` on every `up`, once per replica, even when the bind works.
fn ensure_bind_source(abs: &str) -> BindSource {
	match std::fs::symlink_metadata(abs) {
		Ok(_) => BindSource::Present,
		Err(e) => {
			if e.kind() != std::io::ErrorKind::NotFound {
				return BindSource::Failed(format!(
					"create_host_path: failed to create {abs}: {e}"
				));
			}
			match std::fs::create_dir_all(abs) {
				Ok(()) => BindSource::Created,
				Err(e) => {
					BindSource::Failed(format!("create_host_path: failed to create {abs}: {e}"))
				}
			}
		}
	}
}

/// Build all OCI mounts and named volume attachments for a container.
///
/// Returns `(mounts, named_volumes)`. Named volumes must go into
/// `SpecGenerator.volumes`; bind/tmpfs/npipe mounts go into
/// `SpecGenerator.mounts`.
pub(crate) fn build_mounts_all(
	service: &Service,
	base_dir: &Path,
) -> (Vec<Mount>, Vec<NamedVolume>) {
	let mut mounts = Vec::new();
	let mut named = Vec::new();

	for v in &service.volumes {
		match v {
			VolumeMount::Short(s) => {
				if let Some((m, n)) = parse_volume_string(s) {
					match n {
						Some(nv) => named.push(nv),
						None => {
							let m = m.unwrap();
							// Short-form binds imply `create_host_path` (compose-spec),
							// so create a missing host source directory before mounting.
							// Otherwise `up` aborts with a raw podman HTTP 500 statfs
							// error that leaks the absolute host path. Resolve exactly
							// like the mount source so the directory is created at the
							// path actually bind-mounted (relative anchored to the
							// project dir, leading `~` expanded).
							if let Some(src) = m.source.as_deref() {
								let abs = super::container::resolve_bind_source(src, base_dir);
								if let BindSource::Failed(msg) = ensure_bind_source(&abs) {
									tracing::warn!("{msg}");
								}
							}
							mounts.push(m);
						}
					}
				}
			}
			VolumeMount::Long {
				volume_type,
				source,
				target,
				read_only,
				bind,
				volume,
				tmpfs,
				..
			} => match volume_type {
				VolumeType::Tmpfs => {
					let mut opts: Vec<String> = Vec::new();
					if let Some(t) = tmpfs {
						if let Some(size) = t.size {
							opts.push(format!("size={size}"));
						}
						if let Some(mode) = t.mode {
							opts.push(format!("mode={mode:o}"));
						}
					}
					if read_only.unwrap_or(false) {
						opts.push("ro".into());
					}
					mounts.push(Mount {
						mount_type: "tmpfs".into(),
						source: None,
						destination: target.clone(),
						options: opts,
					});
				}
				VolumeType::Bind => {
					let src = source.as_deref().unwrap_or("");

					if let Some(b) = bind {
						if b.create_host_path.unwrap_or(false) && !src.is_empty() {
							// Resolve exactly like the mount source (expand `~`, anchor a
							// relative path to the project dir) so the directory is created
							// at the path actually bind-mounted, not a literal `~` dir.
							let abs = super::container::resolve_bind_source(src, base_dir);
							if let BindSource::Failed(msg) = ensure_bind_source(&abs) {
								tracing::warn!("{msg}");
							}
						}
					}

					let mut opts = access_opts(*read_only);
					extend_bind_opts_str(&mut opts, bind.as_ref());
					mounts.push(Mount {
						mount_type: "bind".into(),
						source: Some(src.to_string()),
						destination: target.clone(),
						options: opts,
					});
				}
				VolumeType::Volume => {
					let mut opts = access_opts(*read_only);
					extend_volume_opts_str(&mut opts, volume.as_ref());
					named.push(NamedVolume {
						name: source.clone().unwrap_or_default(),
						dest: target.clone(),
						options: opts,
						sub_path: volume.as_ref().and_then(|v| v.subpath.clone()),
					});
				}
				VolumeType::Npipe => {
					mounts.push(Mount {
						mount_type: "npipe".into(),
						source: source.clone(),
						destination: target.clone(),
						options: vec![],
					});
				}
				VolumeType::Cluster => {
					mounts.push(Mount {
						mount_type: "cluster".into(),
						source: source.clone(),
						destination: target.clone(),
						options: vec![],
					});
				}
			},
		}
	}

	// Top-level `tmpfs:` shorthand, equivalent to volumes with type=tmpfs.
	for entry in service.tmpfs.to_list() {
		mounts.push(spec::parse_tmpfs_string(&entry));
	}

	(mounts, named)
}

/// A bind mount referenced from the watcher's bind-mount feedback check.
///
/// `source` is the host path as written (relative paths still need to be
/// resolved against the project base directory, which the caller does with
/// `container::resolve_bind_source`); `target` is the in-container path.
/// `read_only` is the parsed result of the volume spec, so callers can
/// short-circuit the feedback loop on read-only mounts without parsing
/// the same string again: a sync into a read-only bind cannot write back
/// into the watched tree, so it never produces the cycle this check is
/// trying to prevent.
pub(crate) struct BindMountRef {
	pub source: String,
	pub target: String,
	pub read_only: bool,
}

/// The bind mounts declared on `service`, as `(source, target)` pairs with
/// the read-only flag. The source is returned as written; callers resolve
/// it with `container::resolve_bind_source`, as `build_mounts_all` does.
///
/// Unlike `build_mounts_all`, this has no side effects: it never creates a
/// missing host directory, because `watch` calls it on every start.
pub(crate) fn bind_mounts(service: &Service) -> Vec<BindMountRef> {
	let mut out = Vec::new();
	for v in &service.volumes {
		match v {
			VolumeMount::Short(s) => {
				if let Some((Some(mount), None)) = parse_volume_string(s) {
					if let Some(src) = mount.source.as_deref() {
						if !src.is_empty() {
							out.push(BindMountRef {
								source: src.to_string(),
								target: mount.destination,
								read_only: short_form_is_read_only(
									mount.options.iter().map(String::as_str),
								),
							});
						}
					}
				}
			}
			VolumeMount::Long {
				volume_type: VolumeType::Bind,
				source,
				target,
				read_only,
				..
			} => {
				if let Some(src) = source.as_deref() {
					if !src.is_empty() {
						out.push(BindMountRef {
							source: src.to_string(),
							target: target.clone(),
							read_only: read_only.unwrap_or(false),
						});
					}
				}
			}
			_ => {}
		}
	}
	out
}

/// True when the parsed options vector carries an `ro` token as a whole
/// item. The check looks at each token by exact match; `ro,noexec`
/// (with no `,`) does not pass.
fn short_form_is_read_only<'a, I: IntoIterator<Item = &'a str>>(opts: I) -> bool {
	opts.into_iter().any(|o| o == "ro")
}

/// The container-side target of every entry in `service.volumes` (the
/// destination `parse_volume_string` already returns for short form, and
/// `target` for long form), plus every entry of `service.tmpfs` (the
/// part before the first `:`, the same shape
/// `read_only_target_warning` uses). Returned in declaration order;
/// the volume entries first, then the tmpfs entries. Used by the
/// watcher to walk every container destination without re-parsing each
/// string a second time.
pub(crate) fn mount_targets(service: &Service) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	for v in &service.volumes {
		let target = match v {
			VolumeMount::Short(s) => match parse_volume_string(s) {
				Some((Some(mount), _)) => Some(mount.destination),
				Some((None, Some(named))) => Some(named.dest),
				_ => None,
			},
			VolumeMount::Long { target, .. } => Some(target.clone()),
		};
		if let Some(t) = target {
			if !t.is_empty() {
				out.push(t);
			}
		}
	}
	for entry in service.tmpfs.to_list() {
		let dst = entry.split(':').next().unwrap_or("");
		if !dst.is_empty() {
			out.push(dst.to_string());
		}
	}
	out
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
