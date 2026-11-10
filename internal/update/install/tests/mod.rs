//! Tests for the install transaction: the verified swap, the backup, the
//! platform matrix, the version probe and the hostile payloads.
//!
//! They live in their own folder so the production files keep their names for
//! what they hold, and every subject has its tests beside it.

mod backup;
#[cfg(all(test, unix))]
mod hostile;
mod orchestration;
mod platform;
mod self_test;
#[cfg(all(test, unix))]
mod self_test_pipe;
mod swap;
#[cfg(all(test, unix))]
mod swap_hostile;
mod transaction;
