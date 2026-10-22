//! Per-row coverage for the `sensitive_bind_mount` check.

use super::*;
use crate::audit::Finding;

mod access_modes;
mod host_paths;
mod host_root;
mod runtime_paths;
