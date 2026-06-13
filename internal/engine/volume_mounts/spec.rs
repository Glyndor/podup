//! Short-form volume-spec string parsing.
//!
//! Splits `"src:dst[:opts]"` strings (and pre-built secret/config bind strings)
//! into OCI `Mount` / `NamedVolume` parts, handling the Windows drive-letter
//! colon so it is not mistaken for the `src:dst` separator.

use crate::compose::types::{BindOptions, VolumeOptions};
use crate::libpod::types::container::{Mount, NamedVolume};

/// Parse a short-form volume string `"src:dst"` or `"src:dst:opts"`.
///
/// Returns `Some((mount, named))` where exactly one of the two is `Some`.
/// Named volumes go to `SpecGenerator.volumes`; bind mounts go to `mounts`.
pub(super) fn parse_volume_string(s: &str) -> Option<(Option<Mount>, Option<NamedVolume>)> {
	let (src, dst, opts_str) = split_volume_spec(s);
	let opts: Vec<String> = opts_str
		.split(',')
		.map(|o| o.trim().to_string())
		.filter(|o| !o.is_empty())
		.collect();
	if is_bind_source(src) {
		Some((
			Some(Mount {
				mount_type: "bind".into(),
				source: if src.is_empty() {
					None
				} else {
					Some(src.to_string())
				},
				destination: dst.to_string(),
				options: opts,
			}),
			None,
		))
	} else {
		Some((
			None,
			Some(NamedVolume {
				name: src.to_string(),
				dest: dst.to_string(),
				options: opts,
				sub_path: None,
			}),
		))
	}
}

/// Whether `s` begins with a Windows drive-letter prefix (e.g. `C:`), so the
/// colon that follows is part of the path rather than a `src:dst` separator.
fn has_windows_drive_prefix(s: &str) -> bool {
	let b = s.as_bytes();
	b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// Classify a short-form volume source. A leading `/`, `.` or `~` marks a host
/// path bind; a Windows drive prefix (`C:\...`) does too. Anything else is a
/// named volume.
fn is_bind_source(src: &str) -> bool {
	src.starts_with('/')
		|| src.starts_with('.')
		|| src.starts_with('~')
		|| has_windows_drive_prefix(src)
}

/// Split a short-form volume spec into `(src, dst, opts)`. Colons separate the
/// fields, except the colon in a leading Windows drive prefix, which belongs to
/// the source path. The destination is always an in-container (Unix) path, so
/// only the source can carry a drive letter.
fn split_volume_spec(s: &str) -> (&str, &str, &str) {
	let scan_from = if has_windows_drive_prefix(s) { 2 } else { 0 };
	let seps: Vec<usize> = s
		.bytes()
		.enumerate()
		.skip(scan_from)
		.filter(|&(_, b)| b == b':')
		.map(|(i, _)| i)
		.take(2)
		.collect();
	match seps.as_slice() {
		[] => (s, s, ""),
		[a] => (&s[..*a], &s[a + 1..], ""),
		[a, b] => (&s[..*a], &s[a + 1..*b], &s[b + 1..]),
		_ => unreachable!("take(2) yields at most two separators"),
	}
}

/// Parse a pre-built bind string (secret/config) — always produces a bind Mount.
pub(super) fn parse_bind_string(s: &str) -> Option<Mount> {
	let parts: Vec<&str> = s.splitn(3, ':').collect();
	let (src, dst, opts_str) = match parts.len() {
		1 => (parts[0], parts[0], ""),
		2 => (parts[0], parts[1], ""),
		_ => (parts[0], parts[1], parts[2]),
	};
	let opts: Vec<String> = opts_str
		.split(',')
		.map(|o| o.trim().to_string())
		.filter(|o| !o.is_empty())
		.collect();
	Some(Mount {
		mount_type: "bind".into(),
		source: if src.is_empty() {
			None
		} else {
			Some(src.to_string())
		},
		destination: dst.to_string(),
		options: opts,
	})
}

pub(super) fn access_opts(read_only: Option<bool>) -> Vec<String> {
	if read_only.unwrap_or(false) {
		vec!["ro".into()]
	} else {
		vec!["rw".into()]
	}
}

pub(super) fn extend_bind_opts_str(opts: &mut Vec<String>, b: Option<&BindOptions>) {
	let Some(b) = b else { return };
	if let Some(p) = &b.propagation {
		opts.push(p.clone());
	}
	if let Some(s) = &b.selinux {
		opts.push(s.clone());
	}
}

pub(super) fn extend_volume_opts_str(opts: &mut Vec<String>, v: Option<&VolumeOptions>) {
	let Some(v) = v else { return };
	if v.nocopy.unwrap_or(false) {
		opts.push("nocopy".into());
	}
}

#[cfg(test)]
mod tests {
	use super::{is_bind_source, split_volume_spec};

	#[test]
	fn windows_drive_source_is_a_bind_not_a_named_volume() {
		// `C:\data:/in/container` — the drive colon must not be read as the
		// src/dst separator, and the source must classify as a bind.
		assert_eq!(
			split_volume_spec(r"C:\data:/in/container"),
			(r"C:\data", "/in/container", "")
		);
		assert!(is_bind_source(r"C:\data"));
		assert!(is_bind_source("D:/forward/slash"));
	}

	#[test]
	fn unix_volume_split_is_unchanged() {
		assert_eq!(split_volume_spec("vol:/data"), ("vol", "/data", ""));
		assert_eq!(split_volume_spec("./src:/dst:ro"), ("./src", "/dst", "ro"));
		assert_eq!(split_volume_spec("named"), ("named", "named", ""));
		assert!(!is_bind_source("named"));
		assert!(is_bind_source("/abs"));
		assert!(is_bind_source("./rel"));
		assert!(is_bind_source("~/home"));
	}
}
