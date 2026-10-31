//! Build context tar assembly and ignore-file matching (`.containerignore` / `.dockerignore`).
//!
//! Walks the build context directory, applies ignore semantics (last-match-wins
//! with `!` re-includes, `*`/`?`/`**` globs), and packs the result into a
//! gzipped tar suitable for the libpod build endpoint.
//!
//! Podman reads `.containerignore` or `.dockerignore` and prefers its own when
//! both exist; exactly one applies, never the union. The matcher itself lives
//! in [`crate::engine::context::ignore_patterns`] so the watch engine can share it.

use std::path::{Component, Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::engine::context::ignore_patterns::{
	is_ignored, negation_could_reach, to_ignore_path, try_read_patterns,
};
use crate::engine::context::walk;
use crate::error::{ComposeError, Result};

/// Every directory that contains `path`, outermost first: `a/b/c.txt` yields
/// `a` and `a/b`. The file itself is not one of them.
fn ancestor_dirs(path: &str) -> Vec<String> {
	let mut out = Vec::new();
	let mut acc = String::new();
	for segment in path.split('/') {
		if !acc.is_empty() {
			acc.push('/');
		}
		acc.push_str(segment);
		out.push(acc.clone());
	}
	out.pop();
	out
}

/// Append the build context to `tar`, honoring `.dockerignore`.
///
/// `skip_names` are context-relative paths to omit even when not ignored (used to
/// drop the user's `.dockerignore` so it can be rewritten with extra rules).
fn append_context<W: std::io::Write>(
	tar: &mut tar::Builder<W>,
	context: &Path,
	ignore_patterns: &[String],
	skip_names: &[&str],
	force_include: &[&str],
) -> Result<()> {
	// Do not dereference symlinks: a symlink in the context would otherwise pack
	// the bytes of its (possibly out-of-context) target, e.g. `/etc/hostname` or
	// an SSH key, into the image. Store the link itself instead, matching the
	// watch-sync and cp paths.
	tar.follow_symlinks(false);
	// The directories that hold a force-included path. The force-include is
	// applied per entry, but the walk prunes an ignored directory before it
	// looks at what is inside, so a Dockerfile under an ignored directory
	// never reached that check: the tar came out without it and the build
	// failed with "stat .../docker/Dockerfile: no such file or directory".
	let force_dirs: std::collections::HashSet<String> = force_include
		.iter()
		.flat_map(|p| ancestor_dirs(p))
		.collect();
	// Skip directories the ignore patterns would drop wholesale, so the
	// walk does not enumerate every entry under an excluded subtree just
	// to filter each one out (#1746 entry 6). The closure runs once per
	// directory the walk would otherwise recurse into; the existing
	// per-entry `is_ignored` check below remains as the source of truth
	// for the per-file decision, and the directory-level check must agree
	// with it for every child the walk would have produced. A
	// contradiction here would let the walk call return paths the
	// per-entry filter then drops, and the test below pins that the two
	// never diverge for the patterns the engine actually parses.
	let skip_dir = |abs: &std::path::Path| -> bool {
		let rel = match abs.strip_prefix(context) {
			Ok(r) => r,
			Err(_) => return false,
		};
		let rel_str = to_ignore_path(rel);
		// `skip_names` is the only way to drop a directory outright, regardless
		// of ignore patterns (the active `.dockerignore` itself is on the
		// list, so the builder can rewrite it).
		if skip_names.iter().any(|n| rel_str == *n) {
			return true;
		}
		// A directory that holds a force-included path is never pruned. The
		// force-include is applied per entry further down, and pruning takes
		// the entries with it, so the Dockerfile the builder needs would be
		// dropped before that check ever saw it.
		if force_dirs.contains(&rel_str) {
			return false;
		}
		// Pruning is only sound when nothing under this directory can be
		// re-included. `.dockerignore` negation does exactly that:
		//
		//     vendor/
		//     !vendor/keep.txt
		//
		// The directory is ignored and one file under it is not, so pruning
		// `vendor` loses `keep.txt` from the context and a `COPY` naming it
		// fails. The walk's own comment said the engine had no negation
		// patterns "in the wild" that re-include a child; that was an
		// assertion about what users write, and `.dockerignore` supports
		// negation precisely so they can.
		//
		// The cheap, correct condition: descend whenever any negation
		// pattern could name something beneath this directory. The
		// optimisation still applies to the ordinary case, an ignored
		// subtree with nothing re-included, which is what it was for.
		if !is_ignored(&rel_str, ignore_patterns) {
			return false;
		}
		!negation_could_reach(&rel_str, ignore_patterns)
	};
	let include_entry = |abs: &Path| {
		let Ok(rel) = abs.strip_prefix(context) else {
			return false;
		};
		let name = to_ignore_path(rel);
		!skip_names.contains(&name.as_str())
			&& (force_include.contains(&name.as_str()) || !is_ignored(&name, ignore_patterns))
	};
	walk::visit_dir_skipping(context, skip_dir, include_entry, |abs, entry| {
		let rel = abs
			.strip_prefix(context)
			.map_err(|_| std::io::Error::other("build context path strip error"))?;
		match entry {
			walk::Entry::File(mut file) => {
				crate::engine::context::tar_stream::append_pinned_file(tar, rel, &mut file)?;
			}
			walk::Entry::Directory(metadata) => {
				let mut header = tar::Header::new_gnu();
				header.set_metadata(&metadata);
				header.set_entry_type(tar::EntryType::Directory);
				header.set_size(0);
				tar.append_data(&mut header, rel, std::io::empty())?;
			}
			walk::Entry::Symlink(target) => {
				let mut header = tar::Header::new_gnu();
				header.set_entry_type(tar::EntryType::Symlink);
				header.set_mode(0o777);
				header.set_size(0);
				tar.append_link(&mut header, rel, target)?;
			}
		}
		Ok(())
	})
	.map_err(ComposeError::Io)
}

fn append_extra_files<W: std::io::Write>(
	tar: &mut tar::Builder<W>,
	extra_files: &[(String, Vec<u8>)],
) -> Result<()> {
	for (name, bytes) in extra_files {
		let mut header = tar::Header::new_gnu();
		header.set_size(bytes.len() as u64);
		header.set_mode(0o600);
		header.set_cksum();
		tar.append_data(&mut header, name, bytes.as_slice())
			.map_err(|e| ComposeError::Build(e.to_string()))?;
	}
	Ok(())
}

/// The `.dockerfile-inline` entry name synthesized for an inline Dockerfile.
pub(super) const INLINE_DOCKERFILE_NAME: &str = ".dockerfile-inline";

/// The tar entry name reserved for a Dockerfile that lives outside the build
/// context.
///
/// The daemon resolves the `dockerfile` request parameter against the
/// extracted context, so a path that leaves the context names a file that is
/// not there. Such a Dockerfile is read on the client and shipped inside the
/// tar under this name instead.
pub(super) const OUTSIDE_DOCKERFILE_NAME: &str = ".podup-build-dockerfile";

/// Whether `dockerfile` names a path that leaves `context`.
///
/// The comparison is lexical: `.` and `..` components are folded away and the
/// two paths are compared component by component, so a path that stays inside
/// the context is recognised as such without touching the filesystem. An
/// absolute path is left to the daemon, which reads it from its own
/// filesystem.
pub(super) fn dockerfile_leaves_context(context: &Path, dockerfile: &str) -> bool {
	let candidate = Path::new(dockerfile);
	if candidate.is_absolute() {
		return false;
	}
	let context = fold_dot_segments(context);
	let candidate = fold_dot_segments(&context.join(candidate));
	!candidate.starts_with(&context)
}

/// Fold `.` and `..` components away, without touching the filesystem.
fn fold_dot_segments(path: &Path) -> PathBuf {
	let mut out = PathBuf::new();
	for component in path.components() {
		match component {
			Component::CurDir => {}
			Component::ParentDir => {
				out.pop();
			}
			other => out.push(other.as_os_str()),
		}
	}
	out
}

/// Assemble the gzipped build-context tar and write it straight to `writer`.
///
/// This is the shared core of context assembly. `build_context_tar` wraps it
/// to collect the bytes into a `Vec` (tests, non-streaming callers); the build
/// path feeds a channel-backed writer so a multi-gigabyte context is streamed to
/// the socket without ever inflating the process's RSS.
pub(super) fn stream_build_context<W: std::io::Write>(
	writer: W,
	context: &Path,
	dockerfile: &str,
	extra_files: &[(String, Vec<u8>)],
) -> Result<()> {
	let (ignore_name, ignore_patterns) = try_read_patterns(context).map_err(ignore_file_error)?;
	let encoder = GzEncoder::new(writer, Compression::default());
	let mut tar = crate::engine::context::tar_stream::builder(encoder);

	// The names podup synthesizes are reserved: a file in the context
	// carrying one of them would land in the tar next to the synthesized
	// entry, and the daemon extracts the entries in order, so the context
	// file could replace the Dockerfile or the secret the build was asked to
	// use.
	let mut skip: Vec<&str> = extra_files.iter().map(|(n, _)| n.as_str()).collect();
	// Force-include the active Dockerfile so an ignore file that matches it
	// cannot drop it from the context the builder receives (Docker parity).
	// A Dockerfile that ships inside the tar under a reserved name is not
	// force-included: the synthesized entry is the one that must survive.
	let force: Vec<&str> = if extra_files.iter().any(|(n, _)| n.as_str() == dockerfile) {
		Vec::new()
	} else {
		vec![dockerfile]
	};

	if extra_files.is_empty() {
		append_context(&mut tar, context, &ignore_patterns, &skip, &force)?;
	} else {
		// Skip the user's ignore file; it is rewritten with an exclusion per
		// synthesized entry so a `COPY .` in the build cannot bake secret bytes
		// into image layers. It must be skipped and re-emitted under the same
		// name the server will read, or the exclusions never apply.
		skip.push(ignore_name);
		append_context(&mut tar, context, &ignore_patterns, &skip, &force)?;
		let synthesized: Vec<&str> = extra_files.iter().map(|(n, _)| n.as_str()).collect();
		append_ignore_file(
			&mut tar,
			ignore_name,
			&synthesized_ignore_file(context, ignore_name, &synthesized)?,
		)?;
	}
	append_extra_files(&mut tar, extra_files)?;
	finish_tar(tar)
}

/// Pack a directory into a gzipped tar, with no ignore rules.
///
/// The additional build contexts are separate trees the build names as
/// `name`; each one travels in its own part of the request body, and the
/// daemon extracts it on its own. The ignore file of the main context does
/// not apply to them.
pub(super) fn stream_directory_tar<W: std::io::Write>(writer: W, dir: &Path) -> Result<()> {
	let encoder = GzEncoder::new(writer, Compression::default());
	let mut tar = crate::engine::context::tar_stream::builder(encoder);
	append_context(&mut tar, dir, &[], &[], &[])?;
	finish_tar(tar)
}

/// As [`stream_build_context`], but injects an inline Dockerfile as
/// [`INLINE_DOCKERFILE_NAME`] instead of shipping one from the context.
pub(super) fn stream_build_context_with_inline<W: std::io::Write>(
	writer: W,
	context: &Path,
	inline: &str,
	extra_files: &[(String, Vec<u8>)],
) -> Result<()> {
	let (ignore_name, ignore_patterns) = try_read_patterns(context).map_err(ignore_file_error)?;
	let encoder = GzEncoder::new(writer, Compression::default());
	let mut tar = crate::engine::context::tar_stream::builder(encoder);

	let mut header = tar::Header::new_gnu();
	header.set_size(inline.len() as u64);
	header.set_mode(0o644);
	header.set_cksum();
	tar.append_data(&mut header, INLINE_DOCKERFILE_NAME, inline.as_bytes())
		.map_err(|e| ComposeError::Build(e.to_string()))?;

	// Skip the user's ignore file here; it is rewritten below with extra rules
	// excluding every synthesized entry (inline Dockerfile, build secrets), under
	// the same name the server will read.
	//
	// The inline Dockerfile's own name is reserved: a file in the context
	// carrying it would land in the tar after the synthesized entry, and the
	// daemon extracts the entries in order, so that file would replace the
	// Dockerfile the build was asked to use. The synthesized extra files are
	// reserved for the same reason.
	let mut skip: Vec<&str> = vec![ignore_name, INLINE_DOCKERFILE_NAME];
	skip.extend(extra_files.iter().map(|(n, _)| n.as_str()));
	append_context(&mut tar, context, &ignore_patterns, &skip, &[])?;

	let mut synthesized: Vec<&str> = vec![INLINE_DOCKERFILE_NAME];
	synthesized.extend(extra_files.iter().map(|(n, _)| n.as_str()));
	append_ignore_file(
		&mut tar,
		ignore_name,
		&synthesized_ignore_file(context, ignore_name, &synthesized)?,
	)?;

	append_extra_files(&mut tar, extra_files)?;
	finish_tar(tar)
}

/// Finish the gzip stream and flush the sink. `GzEncoder::finish` writes the
/// trailer but does not flush the underlying writer, so a channel-backed writer
/// would strand its last buffered chunk without this explicit flush.
fn finish_tar<W: std::io::Write>(tar: tar::Builder<GzEncoder<W>>) -> Result<()> {
	let gz = tar
		.into_inner()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	let mut writer = gz
		.finish()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	writer
		.flush()
		.map_err(|e| ComposeError::Build(e.to_string()))?;
	Ok(())
}

/// Collect [`stream_build_context_with_inline`] into a `Vec`. Test-only: the
/// build path streams the tar; these wrappers let the tar assembly be asserted
/// on its bytes.
#[cfg(test)]
pub(super) fn build_context_tar_with_inline(
	context: &Path,
	inline: &str,
	extra_files: &[(String, Vec<u8>)],
) -> Result<(Vec<u8>, String)> {
	let mut buf = Vec::new();
	stream_build_context_with_inline(&mut buf, context, inline, extra_files)?;
	Ok((buf, INLINE_DOCKERFILE_NAME.to_string()))
}

/// Collect [`stream_build_context`] into a `Vec`. Test-only (see
/// [`build_context_tar_with_inline`]).
#[cfg(test)]
pub(crate) fn build_context_tar(
	context: &Path,
	dockerfile: &str,
	extra_files: &[(String, Vec<u8>)],
) -> Result<Vec<u8>> {
	let mut buf = Vec::new();
	stream_build_context(&mut buf, context, dockerfile, extra_files)?;
	Ok(buf)
}

/// Append a synthesized `.dockerignore` entry to the context tar.
fn append_ignore_file<W: std::io::Write>(
	tar: &mut tar::Builder<W>,
	name: &str,
	content: &str,
) -> Result<()> {
	let mut header = tar::Header::new_gnu();
	header.set_size(content.len() as u64);
	header.set_mode(0o644);
	header.set_cksum();
	tar.append_data(&mut header, name, content.as_bytes())
		.map_err(|e| ComposeError::Build(e.to_string()))
}

/// Build the ignore-file content for a context tar carrying synthesized entries:
/// any user rules plus a final exclusion per synthesized name, so a `COPY .` in
/// the build does not capture the inline Dockerfile or a build secret. The
/// libpod `secrets=id=…,src=…` mount reads straight from the extracted context,
/// which the ignore file does not filter, so excluded secret entries remain
/// mountable.
///
/// `name` is the ignore file the server will read (see
/// [`crate::engine::context::ignore_patterns::read_patterns`]); the
/// user rules are carried over from that same file, never from the other one.
fn synthesized_ignore_file(context: &Path, name: &str, names: &[&str]) -> Result<String> {
	// A file that was there a moment ago and cannot be read now is an error, not
	// an empty list: the rewritten file replaces the user's rules, so an empty
	// read would ship what they excluded.
	let existing = match crate::filesystem::read_to_string_capped(context.join(name)) {
		Ok(content) => content,
		Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
		Err(e) => return Err(ignore_file_error(e)),
	};
	let mut out = existing.trim_end_matches(['\n', '\r']).to_string();
	for name in names {
		if !out.is_empty() {
			out.push('\n');
		}
		out.push_str(name);
	}
	out.push('\n');
	Ok(out)
}

/// The error for an ignore file that exists and cannot be used. It names the
/// problem and never the file's content.
fn ignore_file_error(e: std::io::Error) -> ComposeError {
	ComposeError::Build(format!(
		"cannot read the build context's ignore file ({}); fix it or remove it so the build does not ship files it was meant to exclude",
		e.kind()
	))
}

/// How a compose `additional_contexts` value reaches the daemon.
pub(super) enum AdditionalKind {
	/// An image the daemon pulls itself.
	Image(String),
	/// A URL the daemon downloads itself.
	Url(String),
	/// A directory the client reads and sends in the request body.
	Local(PathBuf),
}

/// Classify a compose `additional_contexts` value.
///
/// The daemon fetches the image and the URL forms itself. A directory is
/// the client's to read: the endpoint reads a local context from the
/// request body, not from the query.
pub(super) fn additional_context_kind(base_dir: &Path, value: &str) -> AdditionalKind {
	if let Some(img) = value.strip_prefix("docker-image://") {
		AdditionalKind::Image(img.to_string())
	} else if value.starts_with("http://")
		|| value.starts_with("https://")
		|| value.starts_with("git://")
	{
		AdditionalKind::Url(value.to_string())
	} else {
		AdditionalKind::Local(base_dir.join(value))
	}
}

/// Map a compose `additional_contexts` value to the libpod
/// `additionalbuildcontexts` form: `image:`, `url:`, or `localpath:`.
pub(super) fn map_additional_context(base_dir: &Path, value: &str) -> String {
	match additional_context_kind(base_dir, value) {
		AdditionalKind::Image(img) => format!("image:{img}"),
		AdditionalKind::Url(url) => format!("url:{url}"),
		AdditionalKind::Local(path) => format!("localpath:{}", path.display()),
	}
}

#[cfg(test)]
mod tests;
