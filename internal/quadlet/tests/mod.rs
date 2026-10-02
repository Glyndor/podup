use crate::quadlet::{QuadletOutput, QuadletUnit};

mod fields;
mod fields_logging;
mod fields_resources;
mod floor_compat;
mod floor_compat_empty;
mod floor_compat_source;
mod health;
mod network_volume;
mod podman_argv;
mod pull_policy;
mod units;

pub(super) use podman_argv::assert_argv_has_no_token;

fn unit_named<'a>(out: &'a QuadletOutput, filename: &str) -> &'a QuadletUnit {
	out.units
		.iter()
		.find(|u| u.filename == filename)
		.unwrap_or_else(|| panic!("no unit named {filename}"))
}
