//! The multipart request body the libpod build endpoint reads the local
//! additional build contexts from.
//!
//! The endpoint reads the main context from the `MainContext` part and each
//! additional local context from a part named `build-context-NAME`. The
//! contexts the daemon fetches on its own — a `url:` it downloads and an
//! `image:` it pulls — travel in the query instead.
//!
//! A local context in the query is not enough: the endpoint reads the query
//! entries for the URL and the image forms only, so a directory named there
//! was silently dropped and the build could not use the context it was
//! given.

use std::io::Write;
use std::path::PathBuf;

use futures_util::stream::BoxStream;
use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::super::context::{
	stream_build_context, stream_build_context_with_inline, stream_directory_tar,
};
use super::stream::{channel_writer, BodyItem, ContextSource, CHANNEL_CAP};
use crate::error::{ComposeError, Result};

/// The boundary that separates the parts of the multipart body.
///
/// Random per request: a boundary has to be absent from the payload, and
/// the payload is a tar of files the user controls. The value comes from
/// the standard library's randomly seeded hasher, so it costs no
/// dependency.
pub(in crate::backends::engine::images::build) fn boundary() -> String {
	use std::hash::{BuildHasher, Hasher};
	let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
	hasher.write_u32(std::process::id());
	format!("podup-build-{:016x}", hasher.finish())
}

/// The build context the multipart body carries: the directory, what to
/// take from it, and the build secrets to ship inside it.
type MainContext = (PathBuf, ContextSource, Vec<(String, Vec<u8>)>);

/// The multipart body of a build request.
///
/// `main` is the build context, absent when the daemon fetches the context
/// itself. `additional` are the local additional contexts, each packed as
/// its own part.
pub(in crate::backends::engine::images::build) fn multipart_body(
	boundary: String,
	main: Option<MainContext>,
	additional: Vec<(String, PathBuf)>,
) -> (JoinHandle<Result<()>>, BoxStream<'static, BodyItem>) {
	let (tx, rx) = mpsc::channel::<BodyItem>(CHANNEL_CAP);
	let producer = tokio::task::spawn_blocking(move || -> Result<()> {
		let mut writer = channel_writer(tx.clone());
		if let Some((context, source, secrets)) = &main {
			write_part_header(&mut writer, &boundary, "MainContext")?;
			match source {
				ContextSource::Inline(inline) => {
					stream_build_context_with_inline(&mut writer, context, inline, secrets)?
				}
				ContextSource::Dockerfile(dockerfile) => {
					stream_build_context(&mut writer, context, dockerfile, secrets)?
				}
				ContextSource::OutsideContext { name, bytes } => {
					let mut files = secrets.clone();
					files.push((name.clone(), bytes.clone()));
					stream_build_context(&mut writer, context, name, &files)?
				}
			}
			write_part_end(&mut writer)?;
		}
		for (name, dir) in &additional {
			write_part_header(&mut writer, &boundary, &format!("build-context-{name}"))?;
			stream_directory_tar(&mut writer, dir)?;
			write_part_end(&mut writer)?;
		}
		writer
			.write_all(format!("--{boundary}--\r\n").as_bytes())
			.map_err(build_error)?;
		writer.flush().map_err(build_error)?;
		Ok(())
	});

	let body = futures_util::stream::unfold(rx, |mut rx| async move {
		rx.recv().await.map(|item| (item, rx))
	})
	.boxed();

	(producer, body)
}

/// The header of one part: the boundary, the part's name, and the type of
/// the payload it carries.
fn write_part_header<W: Write>(writer: &mut W, boundary: &str, name: &str) -> Result<()> {
	write!(
		writer,
		"--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\
		 Content-Type: application/x-tar\r\n\r\n"
	)
	.map_err(build_error)
}

/// The separator between two parts.
fn write_part_end<W: Write>(writer: &mut W) -> Result<()> {
	writer.write_all(b"\r\n").map_err(build_error)
}

fn build_error(e: std::io::Error) -> ComposeError {
	ComposeError::Build(e.to_string())
}

#[cfg(test)]
mod tests;
