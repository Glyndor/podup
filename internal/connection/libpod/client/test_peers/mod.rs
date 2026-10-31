//! Stand-ins for the Podman side of the client in tests: an in-memory HTTP peer and a closed transport.

#[cfg(all(test, unix))]
pub(crate) mod pre_start_memory;
#[cfg(test)]
pub(in crate::connection::libpod::client) mod preflight;
