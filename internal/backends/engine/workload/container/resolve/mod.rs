//! Name, path, link, and config-hash resolution for container creation.

mod environment;
mod hash;
mod label_files;
#[cfg(test)]
mod scale_hash_tests;
mod signals;
mod volumes;
mod volumes_from;

pub(crate) use environment::{effective_env, unresolved_env_keys};
pub(crate) use hash::config_hash;
pub(super) use signals::resolve_stop_signal;
pub(crate) use volumes::resolve_bind_source;
pub(super) use volumes::resolve_volume_name;
pub(super) use volumes_from::resolve_volumes_from;

#[cfg(test)]
mod resolve_tests;

#[cfg(test)]
mod resolve_file_digest_tests;

#[cfg(test)]
mod label_file_hash_tests;
