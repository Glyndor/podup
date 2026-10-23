//! The per-check run functions: one pure function per audit check id, each
//! returning the findings a service raises under that check. Empty list =
//! passed. Split out of `checks.rs` so the registry (which enumerates them
//! for `audit --list-checks`) and the function bodies stay readable on their
//! own; a new check still adds one entry to the registry plus one function
//! here. The drift test in `audit_tests.rs` pins the registry id against the
//! `&'static str` each function passes to `finding`, so a change that updates
//! one side and not the other cannot land.

use super::Finding;

mod image;
mod limits;
mod namespaces;
mod ports;
mod privileges;
mod root_filesystem;
mod secrets;

pub use image::check_unpinned_image;
pub use limits::{check_no_memory_limit, check_no_pids_limit};
pub use namespaces::{check_host_namespace, check_no_userns};
pub use ports::{check_port_published_on_all_interfaces, check_port_published_on_wildcard};
pub use privileges::{
	check_dangerous_capability, check_no_cap_drop_all, check_no_new_privileges_off,
	check_privileged,
};
pub use root_filesystem::check_writable_root;
pub use secrets::{check_secret_in_environment, segments};

/// Construct one [`Finding`]. Private to this module so callers always go
/// through `run_checks`, and so the field-name ordering (service, check,
/// reason) is consistent across all checks.
pub(super) fn finding(name: &str, check: &'static str, reason: &str) -> Finding {
	Finding {
		service: name.to_string(),
		check,
		reason: reason.to_string(),
	}
}

#[cfg(test)]
mod secrets_tests;

#[cfg(test)]
mod image_tests;

#[cfg(test)]
mod ports_tests;

#[cfg(test)]
mod ports_wildcard_tests;

#[cfg(test)]
mod privileges_tests;

#[cfg(test)]
mod namespaces_tests;

#[cfg(test)]
mod root_filesystem_tests;

#[cfg(test)]
mod limits_tests;
