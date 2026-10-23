//! PUTting a host archive into a container and confirming it landed, the
//! upload path `cp` and the watch sync share: [`put`] performs the PUT, and
//! [`verify`] asks the destination whether every entry that was sent is
//! there.
//!
//! The bytes the PUT carries come from [`super::pack`].

pub(in crate::engine) mod put;
pub(in crate::engine) mod verify;

/// The PUT path builder, under the name the wire-shape test in
/// `engine::lifecycle::libpod_endpoint_query_tests` names it by.
#[cfg(test)]
pub(in crate::engine) use put::archive_put_path;

#[cfg(all(test, unix))]
mod tests;
