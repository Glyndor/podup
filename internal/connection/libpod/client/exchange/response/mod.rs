//! Reading a response: the status-code and version-string helpers, and the
//! decoded path-stat header type.
//!
//! [`status`] turns a non-success status plus the daemon's error body into a
//! `PodmanError`, and gates the reported `Libpod-API-Version` against the
//! floor. [`path_stat`] is the wire shape of
//! `X-Docker-Container-Path-Stat`, which the `HEAD` path-stat calls decode.

use crate::connection::libpod::client::{Client, PodmanError, Result};

pub(in crate::connection::libpod::client) mod path_stat;
pub(in crate::connection::libpod::client) mod status;

#[cfg(test)]
#[path = "status_tests.rs"]
mod status_tests;
