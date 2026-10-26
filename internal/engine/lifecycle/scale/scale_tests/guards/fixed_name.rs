use super::super::*;

#[test]
fn fixed_container_name_single_replica_is_allowed() {
	let svc = service("services:\n  app:\n    image: x\n    container_name: myapp\n");
	assert!(check_fixed_name_scale("app", &svc, 1).is_ok());
}

#[test]
fn fixed_container_name_scaled_above_one_is_rejected() {
	let svc = service("services:\n  app:\n    image: x\n    container_name: myapp\n");
	let err = check_fixed_name_scale("app", &svc, 3).unwrap_err();
	assert!(matches!(err, crate::error::ComposeError::Unsupported(_)));
	assert!(err.to_string().contains("container_name"));
}

#[test]
fn unnamed_service_scales_freely() {
	let svc = service("services:\n  app:\n    image: x\n");
	assert!(check_fixed_name_scale("app", &svc, 5).is_ok());
}
