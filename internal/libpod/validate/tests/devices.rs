use super::*;

#[test]
fn device_access_accepts_rwm_subsets() {
	assert!(is_valid_device_access("r"));
	assert!(is_valid_device_access("w"));
	assert!(is_valid_device_access("m"));
	assert!(is_valid_device_access("rw"));
	assert!(is_valid_device_access("rwm"));
	assert!(is_valid_device_access("wm"));
}

#[test]
fn device_access_rejects_invalid_chars() {
	assert!(!is_valid_device_access("x"));
	assert!(!is_valid_device_access("rwx"));
	assert!(!is_valid_device_access("r w"));
	assert!(!is_valid_device_access("rw "));
}

#[test]
fn first_invalid_device_access_returns_index_and_field() {
	let rules = ["rwm", "rwmx", "rw"];
	let (field, value) = first_invalid_device_access(rules.iter().copied()).unwrap();
	assert_eq!(field, "device_cgroup_rule[1].access");
	assert_eq!(value, "rwmx");
}

#[test]
fn first_invalid_device_access_skips_empty() {
	let rules = ["rwm", "", "rwx"];
	let (field, _) = first_invalid_device_access(rules.iter().copied()).unwrap();
	assert_eq!(field, "device_cgroup_rule[2].access");
}

#[test]
fn first_invalid_device_access_passes_when_all_valid() {
	let rules = ["rwm", "r", "wm"];
	assert!(first_invalid_device_access(rules.iter().copied()).is_none());
}

/// A `device_cgroup_rules:` entry the engine's own parser does not accept
/// as a rule is not validated either. `parse_device_cgroup_rule` returns
/// `None` for `"c 1:3 rwx extra"` (four fields), so the engine drops the
/// rule; the up-front validator reads access strings through that same
/// parser, so it must not reject the third token (`rwx`, whose `x` is not
/// an access character) of a rule that never reaches the runtime.
#[test]
fn pre_validate_spec_passes_a_malformed_device_cgroup_rule() {
	let svc = crate::compose::types::Service {
		device_cgroup_rules: vec!["c 1:3 rwx extra".to_string()],
		..Default::default()
	};
	assert!(pre_validate_spec("web", &svc).is_ok());
}

/// A well-formed `device_cgroup_rules:` entry with an access string outside
/// `r`, `w`, `m` is rejected, and the error names the field and the value.
#[test]
fn pre_validate_spec_rejects_an_invalid_device_cgroup_rule_access() {
	let svc = crate::compose::types::Service {
		device_cgroup_rules: vec!["c 1:3 rwx".to_string()],
		..Default::default()
	};
	match pre_validate_spec("web", &svc) {
		Err(ComposeError::Podman(PodmanError::Field { value, .. })) => {
			assert_eq!(value, "rwx", "the error must carry the offending access");
		}
		other => panic!("an invalid access must be rejected with a field error; got {other:?}"),
	}
}

/// The same rule reaches `devices:` entries, whose access is the third
/// `:`-separated segment. It is read through `device_spec_parts`, the split
/// `parse_device` uses, so a regression in that split that loses the access
/// would let this through.
#[test]
fn pre_validate_spec_rejects_an_invalid_devices_access() {
	let svc = crate::compose::types::Service {
		devices: vec!["/dev/null:/dev/null:xyz".to_string()],
		..Default::default()
	};
	match pre_validate_spec("web", &svc) {
		Err(ComposeError::Podman(PodmanError::Field { value, .. })) => {
			assert_eq!(value, "xyz", "the error must carry the offending access");
		}
		other => panic!("an invalid access must be rejected with a field error; got {other:?}"),
	}
}
