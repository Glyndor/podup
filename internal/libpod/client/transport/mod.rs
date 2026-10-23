//! The byte stream under every libpod connection, and what a failed connect
//! looks like to the operator.
//!
//! [`stream`] is the one place that knows whether this build talks to a Unix
//! socket or to a `podman machine` named pipe, so the hyper request path and
//! the hand-written hijack connect through the same code instead of each
//! carrying its own `cfg` blocks. [`socket`] turns the bare `io::Error` a
//! connect returns into a `PodmanError::Connect` that names the path and the
//! fix (#1146).

use super::{PodmanError, Result};

mod socket;
pub(in crate::libpod::client) mod stream;

#[cfg(all(test, unix))]
#[path = "socket_tests.rs"]
mod socket_tests;

#[cfg(unix)]
pub(crate) use socket::socket_error;
