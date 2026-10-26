//! Reading and interacting with what is running: ps, logs, exec, attach and inspect, the stats stream, the Podman event stream and the interactive terminal they share.

pub(crate) mod events;
pub(crate) mod query;
pub(in crate::engine) mod stats;
pub(crate) mod terminal;
