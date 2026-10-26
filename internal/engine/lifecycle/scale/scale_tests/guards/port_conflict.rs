use super::*;

#[test]
fn single_replica_never_conflicts() {
	let svc = service("services:\n  web:\n    image: x\n    ports:\n      - \"8080:80\"\n");
	assert!(check_scale_port_conflict("web", &svc, 1).is_ok());
}

#[test]
fn scaled_fixed_host_port_conflicts() {
	let svc = service("services:\n  web:\n    image: x\n    ports:\n      - \"8080:80\"\n");
	let err = check_scale_port_conflict("web", &svc, 3).unwrap_err();
	assert!(matches!(
		err,
		crate::error::ComposeError::ScalePortConflict { .. }
	));
	assert!(err.to_string().contains("8080"));
}

#[test]
fn scaled_random_host_port_is_allowed() {
	// A container-only port (`"80"`) gets a runtime-assigned host port per
	// replica, so scaling is fine.
	let svc = service("services:\n  web:\n    image: x\n    ports:\n      - \"80\"\n");
	assert!(check_scale_port_conflict("web", &svc, 3).is_ok());
}

#[test]
fn scaled_no_ports_is_allowed() {
	let svc = service("services:\n  worker:\n    image: x\n");
	assert!(check_scale_port_conflict("worker", &svc, 5).is_ok());
}
