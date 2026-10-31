use crate::quadlet::{QuadletOutput, QuadletUnit};

mod exec;
mod fields;
mod floor_compat;
mod health;
mod network;
mod pull_policy;
mod units;

pub(super) use exec::podman_argv::assert_argv_has_no_token;

fn unit_named<'a>(out: &'a QuadletOutput, filename: &str) -> &'a QuadletUnit {
	out.units
		.iter()
		.find(|u| u.filename == filename)
		.unwrap_or_else(|| panic!("no unit named {filename}"))
}
