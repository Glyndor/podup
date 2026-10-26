//! Name, path, link, and config-hash resolution for container creation.

mod environment;
mod hash;
mod links;
mod signals;
mod volumes;

pub(super) use environment::build_env;
pub(crate) use hash::config_hash;
pub(super) use links::{resolve_links, resolve_volumes_from};
pub(super) use signals::resolve_stop_signal;
pub(crate) use volumes::resolve_bind_source;
pub(super) use volumes::resolve_volume_name;

#[cfg(test)]
mod resolve_tests;

#[cfg(test)]
mod resolve_file_digest_tests;
