//! The unit tests of `render.rs`, one file per subject.
//!
//! The suite was a single `render_tests.rs` over the 300-line soft limit, so
//! it became a folder: this module is the index and the shared helpers, and
//! each leaf holds the scenarios of one subject.

use super::*;

mod command;
mod escaping;
mod exec_words;
mod ports;
mod stem;
mod tmpfs;
mod volumes;
