//! Network creation and per-network options for container specs.
//!
//! `create` pre-creates every declared network and stamps it with the project
//! label, applying the default bridge isolation from `isolation` and the libpod
//! IPAM shapes from `ipam`. On the read side, `names` holds the one rule that
//! turns a compose network key into a host name, `per_network_options` the
//! options a service carries into each network, and `mode` the `network_mode` /
//! declared-networks resolution into a netns.

mod create;
mod ipam;
mod isolation;
mod mode;
mod names;
mod per_network_options;

pub(in crate::engine) use mode::{
	network_mode_options, resolve_network_mode, split_network_mode_options,
};
pub use names::resolve_network_name;
