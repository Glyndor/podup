use crate::quadlet::{QuadletOutput, QuadletUnit};

mod exec;
mod fields;
mod floor_compat;
pub(in crate::backends::quadlet) mod generator;
mod health;
mod image_unit_suffix;
mod mount_colon;
mod network;
mod pull_policy;
#[cfg(unix)]
pub(in crate::backends::quadlet) mod real_env;
mod secret_separators;
mod units;

pub(super) use exec::podman_argv::assert_argv_has_no_token;

fn unit_named<'a>(out: &'a QuadletOutput, filename: &str) -> &'a QuadletUnit {
	out.units
		.iter()
		.find(|u| u.filename == filename)
		.unwrap_or_else(|| panic!("no unit named {filename}"))
}
