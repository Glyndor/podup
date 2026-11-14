//! What the container mounts: volumes, tmpfs and secrets.

mod mounts;
mod secrets;

pub(super) use mounts::{emit_bind_dirs, emit_tmpfs, emit_volumes};
pub(super) use secrets::emit_secrets;
