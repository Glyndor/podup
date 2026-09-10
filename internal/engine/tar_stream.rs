//! The one place a `tar::Builder` is constructed.
//!
//! Three paths pack a tar and hand it to Podman: the build context, `cp`, and
//! the watch sync. All three land in the same server-side reader, so all three
//! need the same archive dialect, and the way to guarantee that is to have a
//! single constructor rather than three that agree today.
//!
//! `tests/tar_builder_single_site.rs` keeps it single.

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
pub(super) fn builder<W: std::io::Write>(writer: W) -> tar::Builder<W> {
	let mut tar = tar::Builder::new(writer);
	tar.sparse(false);
	tar
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "tar_stream_tests.rs"]
mod tests;
