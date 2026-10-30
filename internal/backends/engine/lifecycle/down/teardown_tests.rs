//! Tear-down half of the lifecycle suite: `down` across levels, the
//! `container_rm_path` query-string shape, and image removal.
//!
//! Split out of `up/tests.rs` for the reason `scale/scale_tests.rs` and
//! `lifecycle/drop_recheck_tests.rs` were: that file had reached 487 code lines against a
//! 500-line hard limit, so the pull request that broke the build would have
//! been whichever one next added a lifecycle test, not the one that made the
//! file long.
//!
//! The cut is by subject, not by size. What stays in `up/tests.rs` brings a
//! project up; what moves here takes one down.
//!
//! Same fixture pattern as the rest of the lifecycle suite, including the local
//! `engine_with`, which `scale/scale_tests.rs` also carries rather than sharing: three
//! lines duplicated read better here than a helper exported out of a test module.

#[cfg(unix)]
use crate::engine::lifecycle::Engine;

// Guarded like everything it supports: the `down` fixtures need a unix-socket
// fake libpod, while the `container_rm_path` tests below are pure string
// assertions and run everywhere. Without the guard this helper is dead code on
// Windows, where the whole suite is built with `-D warnings`.
#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

#[cfg(unix)]
mod levels;
mod remove_path;
#[cfg(unix)]
mod resources;
#[cfg(unix)]
mod stop_snapshot;
