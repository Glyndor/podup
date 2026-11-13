//! The checks for a service that reaches the host outside `privileged`,
//! capabilities, namespaces and `volumes:`: host devices, an unconfined
//! `security_opt`, device cgroup rules, a secret, config, env file or build
//! input read from a credentials path, and a relative bind that lands outside
//! the project. `host_paths` resolves paths the way the engine does.

use super::{finding, Finding};

mod bind_scope;
mod devices;
mod host_paths;
mod host_sources;
mod security_opts;

pub use bind_scope::check_bind_outside_project;
pub use devices::{check_device_cgroup_rule, check_host_device};
pub use host_sources::check_sensitive_host_source;
pub use security_opts::check_unconfined_security_opt;

#[cfg(test)]
mod devices_tests;

#[cfg(test)]
mod security_opts_tests;

// Host paths are judged as Linux paths; Windows paths never match.
#[cfg(all(test, unix))]
mod host_sources_tests;

// Host paths are judged as Linux paths; Windows paths never match.
#[cfg(all(test, unix))]
mod bind_scope_tests;
