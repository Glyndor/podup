//! Moving files between the host and containers: the cp command and the develop.watch file sync.

/// `pub(crate)` so the fuzz harness behind the `test-helpers` feature can
/// reach `archive::extract_tar_guarded` without widening the published API:
/// the chain stays `engine → copy → archive` all `pub(crate)` (or stricter),
/// which a third-party crate cannot see.
pub(crate) mod copy;
#[cfg(feature = "watch")]
pub(in crate::engine) mod watch;
