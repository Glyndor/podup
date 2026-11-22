//! The global dry-run gate must refuse writes before touching Podman or disk.
//! Real CLI, real HTTP bytes, isolated HOME and project; no real daemon needed.
//! Snapshots cover final contents, directories, symlinks and modes, not transient
//! writes later undone, timestamps, or files outside the isolated workspace.

#![cfg(unix)]

// Reuse the neighbour's raw HTTP response framing, without changing its parser
// (which discards methods) or runner (which rewrites the compose fixture).
#[allow(dead_code)]
#[path = "hostile_podman/server.rs"]
mod wire;

#[path = "hostile_dry_run/cases.rs"]
mod cases;
#[path = "hostile_dry_run/fixture.rs"]
mod fixture;
#[path = "hostile_dry_run/peer.rs"]
mod peer;
#[path = "hostile_dry_run/responses.rs"]
mod responses;
#[path = "hostile_dry_run/run.rs"]
mod run;
#[path = "hostile_dry_run/snapshot.rs"]
mod snapshot;
