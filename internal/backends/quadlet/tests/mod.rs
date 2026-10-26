use crate::quadlet::{QuadletOutput, QuadletUnit};

mod exec_escaping;
mod exec_generator;
mod fields;
mod floor_compat;
mod health;
mod network_isolation;
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
