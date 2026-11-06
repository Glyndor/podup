//! Tests for the execution layer: the re-check of a lifecycle operation whose
//! response was dropped, and the ordered pre-start containers.
//!
//! They live in their own folder so the production files keep their names for
//! what they hold, and every subject has its tests beside it.

#[cfg(test)]
mod drop_recheck;

#[cfg(all(test, unix))]
mod pre_start;

#[cfg(all(test, unix))]
mod pre_start_image;

#[cfg(all(test, unix))]
mod pre_start_run;

#[cfg(all(test, unix))]
mod pre_start_run_cleanup;

#[cfg(all(test, unix))]
mod support;
