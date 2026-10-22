use super::*;

// --- device parsing ---

#[test]
fn parse_device_host_container_perm() {
	let parsed = parse_device("/dev/null:/dev/zero:rwm");
	assert_eq!(parsed.device.path, "/dev/zero");
	// The trailing permission segment becomes a cgroup access rule.
	let rule = parsed.cgroup_rule.expect("perm should yield a cgroup rule");
	assert!(rule.allow);
	assert_eq!(rule.access.as_deref(), Some("rwm"));
}

#[test]
fn parse_device_same_path_both_sides() {
	let parsed = parse_device("/dev/null");
	assert_eq!(parsed.device.path, "/dev/null");
	// No permission segment → no cgroup rule (Podman defaults to rwm).
	assert!(parsed.cgroup_rule.is_none());
}

#[test]
fn parse_device_two_part() {
	let parsed = parse_device("/dev/null:/dev/xvda");
	assert_eq!(parsed.device.path, "/dev/xvda");
	assert!(parsed.cgroup_rule.is_none());
}

#[test]
fn parse_device_restricted_perm_is_preserved() {
	// `devices: ["/dev/sda:/dev/sda:r"]` must keep the read-only restriction
	// rather than silently becoming rwm on the live up path.
	let parsed = parse_device("/dev/sda:/dev/sda:r");
	assert_eq!(parsed.device.path, "/dev/sda");
	let rule = parsed.cgroup_rule.expect("perm should yield a cgroup rule");
	assert!(rule.allow);
	assert_eq!(rule.access.as_deref(), Some("r"));
	// The rule targets the same node as the device it restricts.
	assert_eq!(rule.device_type, Some(parsed.device.device_type));
	assert_eq!(rule.major, Some(parsed.device.major));
	assert_eq!(rule.minor, Some(parsed.device.minor));
}
