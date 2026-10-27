//! The unit files of service and start mode: their text, and where they are
//! written.

pub(super) mod owner;
pub(super) mod paths;
pub(super) mod service;
pub(super) mod start;

#[cfg(test)]
mod start_tests;
