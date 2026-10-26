//! Building the archive that goes INTO a container: the packers, the
//! streaming transport that carries their bytes, and the walk-and-record
//! helpers the `cp` packer and the watch-sync packer share.
//!
//! The reading half, what extracts an archive that came out, lives in
//! [`super::extract`].

pub(in crate::engine::host_files::copy) mod archive;
pub(in crate::engine) mod common;
pub(in crate::engine) mod transport;
