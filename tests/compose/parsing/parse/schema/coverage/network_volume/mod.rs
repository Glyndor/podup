//! Parse tests for features present in the type system but not previously covered.
//!
//! Split out of `tests/compose/parsing/parse/coverage/network_volume.rs` so each file stays
//! under the repository line limit; this file holds the shared imports and the
//! `mod` lines, and the scenario files inherit the imports with
//! `use super::*`.

use podup::compose::types::*;
use podup::parse_str;

// The per-service network configuration block.
mod network_service;

// The long-form volume mount and its `volume:` options.
mod volume;

// The CPU realtime, count and percent fields.
mod cpu;

// The top-level service fields that take more than one shape or reach the
// device layer.
mod service_fields;

// The `uts` and `cgroup` namespace fields.
mod namespaces;

// The `isolation`, `entitlements` and `sbom` build fields.
mod build;

// The `ipam` options of a network.
mod ipam;

// `deploy.labels` in its list and map shapes.
mod deploy_labels;
