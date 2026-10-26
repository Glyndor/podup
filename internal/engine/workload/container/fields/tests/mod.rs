//! Tests for `internal/engine/workload/container/fields/mod.rs`.
//!
//! Split out of `fields.rs` to keep that file within the source line limit.

use super::*;
use crate::compose::types::Service;

mod blkio;
mod devices;
mod encode_path;
mod label_file;
mod resolve_labels;
mod sanitize_kv_pair;
mod swarm_deploy;

fn default_service() -> Service {
	Service::default()
}
