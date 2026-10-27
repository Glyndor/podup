//! Field-level parse tests (split into part files for the line limit).
//!
//! Split out of `tests/compose/parsing/parse/schema/fields/extends_build/mod.rs` so each file stays under
//! the repository line limit; this file holds the shared imports and the `mod`
//! lines, and the scenario files inherit the imports with `use super::*`.

use podup::compose::types::*;
use podup::parse_str;

// New: extends, configs, annotations, scale
mod extends_configs_scale;

// Complex volumes: the long-form bind, tmpfs and volume mounts.
mod volumes;

// Build extras: cache_from, shm_size, network and platforms.
mod build;

// Network maps: the per-service map form and the Podman network modes.
mod networks;

// Healthcheck disable, explicit and through `test: ["NONE"]`.
mod healthcheck;
