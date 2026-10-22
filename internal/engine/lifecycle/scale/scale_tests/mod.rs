//! Tests for the scale module: surplus-replica reconciliation, the
//! `running_replica_names` helper, the replica-limit / port-conflict /
//! fixed-name guard helpers, the bulk `live_project_replicas` listing that
//! powers the per-service lifecycle commands (#1363), and the sorted
//! `live_project_replicas_sorted` variant the per-replica query paths
//! (`exec`/`cp`/`port`/`logs`) share so a single container-list round-trip
//! powers them all (#1445).
//!
//! Same fixture pattern as the rest of the lifecycle suite: a unix-socket
//! fake libpod so the request shape (`all=true`, no `status=` filter) and
//! the per-service slice can both be pinned against a known server.

use super::{
	check_fixed_name_scale, check_replica_limit, check_scale_port_conflict, state_is_active,
	DEFAULT_MAX_REPLICAS,
};

/// One test in this file owns `PODUP_MAX_REPLICAS` for its body; without a
/// lock, `cargo test`'s parallel execution lets the new
/// `up/up_one_tests.rs` fixtures (which drive `up()` with three replicas) read
/// the `set_var(2)` from this test mid-`up()` and fail on
/// `check_replica_limit`. The lock is shared with `up/up_one_tests.rs` so the
/// two files serialise on the env var.
pub(in crate::engine::lifecycle) static MAX_REPLICAS_TEST_LOCK: std::sync::Mutex<()> =
	std::sync::Mutex::new(());

#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::engine::Engine;
#[cfg(unix)]
use crate::error::ComposeError;

#[cfg(unix)]
fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

mod bulk_listing;
mod fixed_name;
mod live_replicas;
mod port_conflict;
mod replica_limit;
mod running_replicas;
mod state;
mod surplus;

fn service(yaml: &str) -> crate::compose::types::Service {
	let file = crate::parse_str(yaml).unwrap();
	file.services.into_iter().next().unwrap().1
}
