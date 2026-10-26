//! How the container is reached and how it reaches others: networks, published
//! and exposed ports, and host entries.

mod host;
mod networks;
mod ports;

pub(super) use host::emit_host_config;
pub(crate) use networks::is_rootless_user_mode;
pub(super) use networks::{emit_network_extras, emit_network_mode, emit_networks};
pub(super) use ports::{emit_expose, emit_ports};
