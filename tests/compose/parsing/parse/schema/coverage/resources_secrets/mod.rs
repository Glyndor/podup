//! Parse tests for features present in the type system but not previously covered.
//!
//! Split out of `tests/compose/parsing/parse/schema/coverage/resources_secrets/mod.rs` so each file stays
//! under the repository line limit; this file holds the shared imports and the
//! `mod` lines, and the scenario files inherit the imports with
//! `use super::*`.

use podup::compose::types::*;
use podup::parse_str;

// The `blkio_config` block: weight, weight_device and the per-device rates.
mod blkio;

// The `gpus` shorthand and the long-form device-reservation objects.
mod gpus;

// `deploy.resources.reservations.devices`.
mod deploy_devices;

// `develop.watch` and its three actions.
mod develop_watch;

// The `post_start` and `pre_stop` lifecycle hooks.
mod lifecycle_hooks;

// `env_file` in its long form, with `required`, `format` and a mixed list.
mod env_file;

// The top-level `secrets:` section and the long-form service secret reference.
mod secrets;
