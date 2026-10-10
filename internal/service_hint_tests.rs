use super::*;

fn compose() -> ComposeFile {
	serde_yaml::from_str(
		r#"
name: logsrepro
services:
  web: {image: nginx:alpine}
  named: {image: nginx:alpine, container_name: mi-contenedor}
"#,
	)
	.unwrap()
}

#[test]
fn explicit_container_name_points_to_service() {
	let err = ComposeError::ServiceNotFound("mi-contenedor".into());
	assert_eq!(
		hint_for(&err, &compose(), "logsrepro"),
		Some("hint: 'mi-contenedor' is the container of service 'named'; pass the service name instead".into())
	);
}

#[test]
fn replica_names_point_to_service() {
	for (name, expected) in [
		("logsrepro-web-1", "hint: 'logsrepro-web-1' is the container of service 'web'; pass the service name instead"),
		("logsrepro-web-12", "hint: 'logsrepro-web-12' is the container of service 'web'; pass the service name instead"),
	] {
		let err = ComposeError::ServiceNotFound(name.into());
		assert_eq!(hint_for(&err, &compose(), "logsrepro").as_deref(), Some(expected));
	}
}

#[test]
fn unmatched_names_have_no_hint() {
	for name in [
		"nope",
		"logsrepro-web-",
		"logsrepro-web-1x",
		"other-web-1",
		"logsrepro-web",
		"logsrepro-web-١",
		"logsrepro_web_1",
		"logsreproweb-1",
	] {
		let err = ComposeError::ServiceNotFound(name.into());
		assert_eq!(hint_for(&err, &compose(), "logsrepro"), None, "{name}");
	}
}

#[test]
fn other_errors_have_no_hint() {
	let err = ComposeError::NotRunning("mi-contenedor".into());
	assert_eq!(hint_for(&err, &compose(), "logsrepro"), None);
}

#[test]
fn hyphenated_project_and_services_match_exactly() {
	let file = serde_yaml::from_str(
		r#"
services:
  api: {image: nginx:alpine}
  api-v2: {image: nginx:alpine}
"#,
	)
	.unwrap();
	for (name, expected) in [
		("my-app-api-v2-1", "hint: 'my-app-api-v2-1' is the container of service 'api-v2'; pass the service name instead"),
		("my-app-api-3", "hint: 'my-app-api-3' is the container of service 'api'; pass the service name instead"),
	] {
		let err = ComposeError::ServiceNotFound(name.into());
		assert_eq!(hint_for(&err, &file, "my-app").as_deref(), Some(expected));
	}
}

#[test]
fn control_characters_in_both_names_are_escaped() {
	let file = serde_yaml::from_str(
		r#"
services:
  "\e[32mnamed": {image: nginx:alpine, container_name: "\e[31mx"}
"#,
	)
	.unwrap();
	let err = ComposeError::ServiceNotFound("\x1b[31mx".into());
	let hint = hint_for(&err, &file, "logsrepro").unwrap();
	assert_eq!(
		hint,
		"hint: '\\u{1b}[31mx' is the container of service '\\u{1b}[32mnamed'; pass the service name instead"
	);
	assert!(!hint.contains('\x1b'));
}

#[test]
fn explicit_container_name_wins_over_replica_match() {
	let file = serde_yaml::from_str(
		r#"
services:
  web: {image: nginx:alpine}
  named: {image: nginx:alpine, container_name: logsrepro-web-1}
"#,
	)
	.unwrap();
	let err = ComposeError::ServiceNotFound("logsrepro-web-1".into());
	assert_eq!(
		hint_for(&err, &file, "logsrepro"),
		Some("hint: 'logsrepro-web-1' is the container of service 'named'; pass the service name instead".into())
	);
}

#[test]
fn duplicate_explicit_names_follow_compose_order() {
	let file = serde_yaml::from_str(
		r#"
services:
  zebra: {image: nginx:alpine, container_name: shared}
  alpha: {image: nginx:alpine, container_name: shared}
"#,
	)
	.unwrap();
	let err = ComposeError::ServiceNotFound("shared".into());
	assert_eq!(
		hint_for(&err, &file, "logsrepro"),
		Some(
			"hint: 'shared' is the container of service 'zebra'; pass the service name instead"
				.into()
		)
	);
}

#[test]
fn wrapped_service_not_found_has_the_same_hint() {
	let err =
		ComposeError::DependencyNotReady(std::sync::Arc::new(ComposeError::DependencyNotReady(
			std::sync::Arc::new(ComposeError::ServiceNotFound("mi-contenedor".into())),
		)));
	assert_eq!(
		hint_for(&err, &compose(), "logsrepro"),
		Some("hint: 'mi-contenedor' is the container of service 'named'; pass the service name instead".into())
	);
}
