//! Unit and engine-level tests for the `pull` path: dependency-closure
//! expansion, `pull_policy` resolution, and the de-duplication that collapses
//! a shared image into a single request.
//!
//! Split out of `build/pull/mod.rs` so the production code stays under the
//! source-line limit, the same way `pull_typo_tests.rs` already is.

use super::policy::libpod_pull_policy;
use super::pull_dep_closure;

#[cfg(unix)]
use crate::engine::fake_podman;

mod dedup;
mod in_band_error;
mod pull_policy;
mod targets;
