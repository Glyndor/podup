//! Wire-shape tests for the query-string compensations the libpod endpoint
//! shape requires.
//!
//! The Docker compat handler and the libpod handler share the URL path but
//! differ on which query keys they read. podup used to talk to libpod
//! over the absolute-form request line, which routed every call to the
//! Docker compat handlers, where the docker-side query keys were the
//! ones in scope. Switching the request line to origin form moves
//! every call to the libpod handlers, which read a different subset:
//! `timeout=` (not `t=`), `volumes=` (not `v=`), `copyUIDGID=false`
//! (the default flips), `ps_args=-ef` (the default flips), and
//! `layers=true` on the build endpoint (the default flips). Each
//! compensation here asserts that the call carries the libpod key and
//! not the docker key, on the wire shape captured by the fake podman
//! the build/lifecycle tests already use.

#![cfg(unix)]

use crate::engine::fake_podman::{self, FakeReply};
use crate::engine::Engine;
use crate::libpod::API_PREFIX;
use std::sync::{Arc, Mutex};

fn engine_with(client: crate::libpod::Client, project: &str) -> Engine {
	Engine::with_base_dir(client, project.into(), std::env::temp_dir())
}

mod build;
mod copy;
mod kill;
mod remove;
mod stop_restart;
mod top;
