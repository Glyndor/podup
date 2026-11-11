//! Tests for the `up` lifecycle: the image decision, the prefetch, the
//! readiness poller, the per-service orchestration, and the guards that run
//! before anything is created.
//!
//! They live in their own folder so the production files keep their names for
//! what they hold, and every subject has its tests beside it.

/// Tests for the pull decision in `images.rs`.
///
/// They drive a whole `up`, so they would sit as naturally in the lifecycle
/// suite, but that file was already at 487 of its 500 code lines, and adding
/// them there put it over. They belong next to the decision they pin anyway,
/// the same way `prefetch.rs` keeps its own.
#[cfg(test)]
#[cfg(unix)]
mod images;

#[cfg(all(test, unix))]
mod bind_required;

#[cfg(all(test, unix))]
mod cleanup_drain;

#[cfg(all(test, unix))]
mod collision;

#[cfg(all(test, unix))]
mod pull_policy_build;

#[cfg(test)]
mod keep_id_create;

#[cfg(all(test, unix))]
mod optional_dependency;

#[cfg(test)]
mod prefetch;

#[cfg(all(test, unix))]
mod readiness;

#[cfg(all(test, unix))]
mod up_build_board;

#[cfg(all(test, unix))]
mod up_env_hash;

#[cfg(all(test, unix))]
mod up_one;

#[cfg(test)]
mod up_orchestration;

#[cfg(all(test, unix))]
mod up_replica;

#[cfg(test)]
mod up_validation;
