//! Whether a command tells you what it did.
//!
//! The sibling of `output_contract.rs`: that file pins the *shape* of output,
//! this one pins that there is any. #1248 found six lifecycle commands exiting
//! 0 in complete silence on a project that was never created, a `push` that
//! uploaded an image and wrote zero bytes, an `up` that created three resources
//! and named none of them, and a `down` that announced destroying a data volume
//! which had never existed.
//!
//! Silence and a false report are the two failure modes, and each needs an
//! acceptance twin: a test that only asserts a note appears is equally
//! satisfied by a note that always appears.
//!
//! This crate root declares the shared `harness` module and one `mod` line per
//! scenario. The rows themselves live in the files declared below.

mod harness;

#[path = "reporting_contract/push.rs"]
mod push;

#[path = "reporting_contract/lifecycle_reporting.rs"]
mod lifecycle_reporting;

#[path = "reporting_contract/wait_output.rs"]
mod wait_output;

#[path = "reporting_contract/progress_board.rs"]
mod progress_board;

#[path = "reporting_contract/pull_reporting.rs"]
mod pull_reporting;
