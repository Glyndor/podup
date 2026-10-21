//! The decoded `X-Docker-Container-Path-Stat` header: a container path's
//! name, size, Go file `mode`, `mtime` and `linkTarget`.
//!
//! Split from `mod.rs` so the type and its long observability-laden doc sit
//! together. The `X-Docker-Container-Path-Stat` header is the wire shape; the
//! runtime's JSON keys live here.

/// The decoded `X-Docker-Container-Path-Stat` header: a container path's name,
/// size, Go file `mode`, `mtime` and `linkTarget`.
///
/// `mtime` is an RFC3339 string compared only for equality, never parsed into a
/// time. **Podman 6 reports it to whole seconds**: `2026-08-03T18:36:05Z`, no
/// fractional part, measured on `podman-6.0.1-1.fc45`, which is why `size` is
/// carried here too: two writes inside one second are indistinguishable by mtime
/// alone. The runtime's JSON uses lowercase keys.
///
/// `linkTarget` is the link's target normalized against the directory the link
/// lives in (absolute paths stay absolute; relative paths are joined to the
/// directory lexically and the `.` / `..` resolved without touching the
/// filesystem; a dangling relative link reports a target that may not exist).
/// The field is empty for non-symlink entries and on runtimes that do not send
/// it; `#[serde(default)]` keeps those answers valid `PathStat`s. The runtime's
/// JSON key is `linkTarget`, hence the rename.
#[derive(serde::Deserialize, Default, Clone, PartialEq, Eq, Debug)]
pub(crate) struct PathStat {
	#[serde(default)]
	pub(crate) size: u64,
	#[serde(default)]
	pub(crate) mode: u64,
	#[serde(default)]
	pub(crate) mtime: String,
	#[serde(default, rename = "linkTarget")]
	pub(crate) link_target: String,
}
