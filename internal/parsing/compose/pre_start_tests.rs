use super::parse_str;

#[test]
fn pre_start_is_retained_without_ignored_field_warning() {
	let file =
		parse_str("services:\n  web:\n    image: base\n    pre_start:\n      - command: [init]\n")
			.unwrap();
	let service = &file.services["web"];
	assert!(!service.unknown.contains_key("pre_start"));
	let value = serde_yaml::to_value(service).unwrap();
	assert_eq!(value["pre_start"][0]["command"][0].as_str(), Some("init"));
}

#[test]
fn pre_start_quadlet_reports_omission_without_exec_start_pre() {
	let file = parse_str("services:\n  web:\n    image: base\n    pre_start: [{command: init}]\n")
		.unwrap();
	let output = crate::quadlet::generate(&file, "project");
	assert!(output
		.warnings
		.iter()
		.any(|w| w.contains("pre_start is not exported")));
	assert!(output
		.units
		.iter()
		.map(|unit| &unit.contents)
		.all(|unit| !unit.contains("ExecStartPre=")));
	assert!(!super::diagnostics::collect(&file)
		.iter()
		.any(|w| w.contains("pre_start")));
}

#[test]
fn pre_start_extends_preserves_step_order_and_duplicates() {
	let file = parse_str(
		r#"
services:
  base:
    image: base
    pre_start: [{command: one}]
  web:
    extends: base
    pre_start: [{command: two}, {command: two}]
"#,
	)
	.unwrap();
	let value = serde_yaml::to_value(&file.services["web"]).unwrap();
	let commands: Vec<_> = value["pre_start"]
		.as_sequence()
		.unwrap()
		.iter()
		.map(|s| s["command"].as_str().unwrap())
		.collect();
	assert_eq!(commands, ["one", "two", "two"]);
}

#[test]
fn pre_start_imported_paths_follow_the_defining_directory() {
	let mut service: super::types::Service = serde_yaml::from_str(
		r#"
pre_start:
  - build: ./init
    env_file: init.env
    volumes: [./seed:/seed]
"#,
	)
	.unwrap();
	let source = std::path::Path::new("/source");
	super::anchor::anchor_service(&mut service, source);
	let value = serde_yaml::to_value(&service).unwrap();
	let step = &value["pre_start"][0];
	let build = source.join("./init").to_string_lossy().into_owned();
	assert_eq!(step["build"].as_str(), Some(build.as_str()));
	let env = source.join("init.env").to_string_lossy().into_owned();
	assert_eq!(step["env_file"].as_str(), Some(env.as_str()));
	let volume = format!("{}:/seed", source.join("./seed").display());
	assert_eq!(step["volumes"][0].as_str(), Some(volume.as_str()));
}
