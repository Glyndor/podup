//! The per-field rules a compose file has to meet before anything runs: memory
//! amounts, CPU limits, replica counts, resource names and volume attributes.
//! Each one fails the read naming the service and the key.

pub(super) mod cpus;
pub(super) mod memory;
pub(super) mod names;
pub(super) mod replicas;
pub(super) mod volumes;
