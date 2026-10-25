//! A hostile Podman, seen from the command line.
//!
//! Each case stands up a fake socket that answers the way a broken or hostile
//! daemon would (endless headers, lying lengths, malformed chunking, nested or
//! truncated JSON, oversized frames, silence) and runs the real `podup` binary
//! against it under an address-space limit. The process must end by itself,
//! without a panic, without a signal and inside a memory ceiling.
//!
//! The fuzz targets `stream_frame` and `stream_ndjson` cover the parsers with
//! random bytes; this covers the whole command with chosen ones.

// Unix sockets, `ulimit` and `/proc` are the whole harness; the client's
// Windows transport is a named pipe and has no counterpart here.
#![cfg(unix)]

#[path = "hostile_podman/endpoints.rs"]
mod endpoints;
#[path = "hostile_podman/run.rs"]
mod run;
#[path = "hostile_podman/server.rs"]
mod server;
#[path = "hostile_podman/streams.rs"]
mod streams;
#[path = "hostile_podman/wire.rs"]
mod wire;
