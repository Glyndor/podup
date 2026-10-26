//! What the compose file declares, read before or without acting on it: host
//! binding modes, pull policy, replica counts, and the autostart start-mode
//! readings.

pub(in crate::engine) mod host_modes;
mod pull_preflight;
pub(in crate::engine) mod replicas;
mod start_mode;
