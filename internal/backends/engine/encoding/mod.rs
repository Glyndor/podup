//! Turning engine data into bytes: JSON for libpod query parameters and NDJSON
//! rows, and log frames onto an output writer.

pub(super) mod json;
pub(super) mod log_frame;
