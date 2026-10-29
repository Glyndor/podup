//! The one place a `tar::Builder` is constructed.
//!
//! Three paths pack a tar and hand it to Podman: the build context, `cp`, and
//! the watch sync. All three land in the same server-side reader, so all three
//! need the same archive dialect, and the way to guarantee that is to have a
//! single constructor rather than three that agree today.
//!
//! `tests/repository/repo_rules/tar_builder_single_site.rs` keeps it single.

/// A `tar::Builder` writing the dialect Podman's archive reader accepts.
///
/// `tar::Builder::new` enables sparse-file support by default. With it on, a
/// file that has holes on disk is written as a GNU sparse entry, whose typeflag
/// is `b'S'`, decimal 83. Podman refuses that type and the whole transfer dies
/// on account of one file. Podman's own client packs in Go and never emits the
/// type, which is why a context `podman build` accepts could be one
/// `podup build` rejected (#1775).
///
/// The refusal has two spellings, measured on Podman 5.7.0, and a search for
/// either one alone misses half the damage:
///
/// - the build endpoint answers `unhandled tar header type 83`, from
///   `containers/storage/pkg/archive`, and `podup build` exits 1;
/// - the archive endpoint that `cp` and the watch sync both PUT to answers
///   `unrecognized Typeflag S`. `cp` exits 1; `watch` only logs
///   `watch action failed`, keeps running and never delivers the file, so
///   there the loss is silent.
///
/// Nobody puts a sparse file in a build context on purpose. They arrive on
/// their own, as database files, disk images, virtual machine state, or
/// anything preallocated with `fallocate`, and `ls` shows nothing unusual, so
/// the failure reads as arbitrary.
///
/// Storing the holes expanded costs bytes on the wire and nothing in memory:
/// every caller wraps the writer in a `GzEncoder`, where a run of zeroes
/// compresses to almost nothing, and the build path streams to the socket
/// instead of buffering the archive.
pub(in crate::backends::engine) fn builder<W: std::io::Write>(writer: W) -> tar::Builder<W> {
	let mut tar = tar::Builder::new(writer);
	tar.sparse(false);
	tar
}

/// Append `file` to `tar` as an entry of exactly the size its header states.
///
/// `tar::Builder::append_file` writes the header from the size `stat` reported
/// and then copies the file until it ends. A file that grows between the two
/// (a log, a database being written) leaves a body longer than the header says,
/// and the reader takes the extra bytes for the next header: the whole archive
/// is corrupt. Here the body is cut at the stated size, and padded with zeroes
/// if the file shrank, so the archive stays well formed either way.
///
/// Returns that size. The caller that records what it sent uses it and does not
/// ask the file again, since a second answer can differ from the first.
pub(in crate::backends::engine) fn append_pinned_file<W: std::io::Write>(
	tar: &mut tar::Builder<W>,
	name: &std::path::Path,
	file: &mut std::fs::File,
) -> std::io::Result<u64> {
	let metadata = file.metadata()?;
	append_exact(tar, name, &metadata, file)
}

fn append_exact<W: std::io::Write, R: std::io::Read>(
	tar: &mut tar::Builder<W>,
	name: &std::path::Path,
	metadata: &std::fs::Metadata,
	body: R,
) -> std::io::Result<u64> {
	let size = metadata.len();
	let mut header = tar::Header::new_gnu();
	header.set_metadata(metadata);
	header.set_size(size);
	tar.append_data(&mut header, name, Exact(body.take(size)))?;
	Ok(size)
}

/// Yields exactly the bytes its inner `Take` allows: the source's own, then
/// zeroes if the source ended early.
struct Exact<R>(std::io::Take<R>);

impl<R: std::io::Read> std::io::Read for Exact<R> {
	fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
		let read = self.0.read(buf)?;
		if read > 0 || self.0.limit() == 0 {
			return Ok(read);
		}
		let pad = (buf.len() as u64).min(self.0.limit()) as usize;
		buf[..pad].fill(0);
		self.0.set_limit(self.0.limit() - pad as u64);
		Ok(pad)
	}
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tar_stream_tests.rs"]
mod tests;
