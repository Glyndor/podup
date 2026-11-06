//! The unit tests of `create.rs`, one file per subject.
//!
//! The suite was a single `create_tests.rs` over the 300-line soft limit, so
//! it became a folder: this module keeps the shared imports and the index,
//! and each leaf holds the scenarios of one subject. Every scenario drives a
//! fake libpod over a unix socket, so the leaves are compiled only there.

#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::engine::secrets::tests_support::{engine_on, file_with_content_secrets};

#[cfg(unix)]
mod environment;
#[cfg(unix)]
mod mode;
#[cfg(unix)]
mod replace;
#[cfg(unix)]
mod sizes;
