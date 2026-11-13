use podup::compose::types::{ComposeFile, Service};

use super::{finding, Finding};

/// `devices:` passes a host device node into the container. Every entry is a
/// finding: a GPU, a serial port or `/dev/fuse` can each be the reason the
/// service exists, but each one is also host hardware the container reaches
/// directly, which the operator should see named in the report. The device
/// path is not judged; `/dev/kvm` and `/dev/sda` are both host devices.
pub fn check_host_device(name: &str, service: &Service, _file: &ComposeFile) -> Vec<Finding> {
	service
		.devices
		.iter()
		.map(|device| {
			finding(
				name,
				"host_device",
				&format!("devices: {device} passes a host device into the container"),
			)
		})
		.collect()
}

/// `device_cgroup_rules:` widens the device allowlist of the container's
/// cgroup. A rule does not create a device node by itself, but it lets the
/// container open one it can reach or create (`mknod` with `CAP_MKNOD`), and
/// `a *:* rwm` allows every device on the host. Every entry is a finding, for
/// the same reason as `devices:`.
pub fn check_device_cgroup_rule(
	name: &str,
	service: &Service,
	_file: &ComposeFile,
) -> Vec<Finding> {
	service
		.device_cgroup_rules
		.iter()
		.map(|rule| {
			finding(
				name,
				"device_cgroup_rule",
				&format!(
					"device_cgroup_rules: {rule} lets the container use host devices \
					 beyond the default set"
				),
			)
		})
		.collect()
}
