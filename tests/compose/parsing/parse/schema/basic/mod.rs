//! One test per compose key, asserting the key reaches the typed field the
//! rest of the code reads.
//!
//! Split out of `tests/compose/parsing/parse/schema/basic/mod.rs` so each file stays under the repository
//! line limit; this file holds the shared imports and the `mod` lines, and the
//! scenario files inherit the imports with `use super::*`.

use podup::compose::types::*;
use podup::parse_str;

// The document itself: a minimal service, an empty map, malformed YAML.
mod document;

// `environment`, as a list and as a map.
mod environment;

// `command` and `entrypoint`, in shell and exec form.
mod commands;

// `ports`, `networks` and `expose`.
mod networking;

// `volumes`, `volumes_from` and `tmpfs`.
mod volumes;

// `healthcheck`, `depends_on`, `hostname`, `domainname`, `mac_address` and
// `read_only`.
mod service_config;

// The top-level `secrets:` section and the service keys that touch the host:
// `security_opt`, `devices`, `group_add`, `userns_mode`, `shm_size`,
// `cgroup_parent`.
mod secrets_and_system;

// `cpu_shares`, `cpuset`, `mem_limit` and `cpus`.
mod cpu_limits;
