//! Table headers, JSON keys and the styling of a command's output are a
//! contract with users' scripts and eyes.
//!
//! #1082's closing observation is that nothing protected them: not one test
//! asserted `NAME`, `STATUS`, or an `images` JSON key, so every drift in that
//! issue (`ConfigFiles` always empty, `top` emitting `null`, `volumes`
//! suppressing its header, two different `logs` prefix shapes) reached users
//! before anyone noticed. Fixing them one by one only resets the clock; this is
//! the net underneath.
//!
//! Deliberately narrow: presence and shape, never values. A test that pinned
//! actual container names would fail for the wrong reason and get deleted.
//!
//! Whether a command reports what it *did* lives in `reporting_contract.rs`.
//!
//! This crate root declares the shared `harness` module and one `mod` line per
//! scenario. The rows themselves live in the files declared below.

mod harness;

#[path = "output_contract/table_headers.rs"]
mod table_headers;

#[path = "output_contract/json_keys.rs"]
mod json_keys;

#[path = "output_contract/version_output.rs"]
mod version_output;

#[path = "output_contract/styling.rs"]
mod styling;

#[path = "output_contract/identifiers.rs"]
mod identifiers;
