use super::build_steps;
use crate::compose::{parse_str, types::Command};

#[test]
fn pre_start_inherits_collections_and_replaces_scalars() {
	let file = parse_str(
		r#"
services:
  web:
    image: base
    user: parent
    working_dir: /app
    environment: {KEEP: yes, CHANGE: old}
    volumes: [data:/data, cache:/cache]
    cap_drop: [ALL]
    networks: [back]
    restart: always
    pre_start:
      - image: init
        user: child
        command: [first]
        environment: {CHANGE: new, EXTRA: value}
        volumes: [other:/data]
        cap_add: [CHOWN]
        networks: [front]
      - command: [second]
"#,
	)
	.unwrap();
	let steps = build_steps(&file.services["web"]);
	assert_eq!(steps.len(), 2);
	let first = &steps[0];
	assert_eq!(first.image.as_deref(), Some("init"));
	assert_eq!(first.user.as_deref(), Some("child"));
	assert_eq!(first.working_dir.as_deref(), Some("/app"));
	let env = first.environment.to_map();
	assert_eq!(env["KEEP"].as_deref(), Some("yes"));
	assert_eq!(env["CHANGE"].as_deref(), Some("new"));
	assert_eq!(env["EXTRA"].as_deref(), Some("value"));
	assert_eq!(
		serde_yaml::to_value(&first.volumes).unwrap(),
		serde_yaml::from_str::<serde_yaml::Value>("[other:/data, cache:/cache]").unwrap()
	);
	assert_eq!(first.cap_drop, ["ALL"]);
	assert_eq!(first.cap_add, ["CHOWN"]);
	assert_eq!(first.networks.names(), ["back", "front"]);
	assert_eq!(steps[1].image.as_deref(), Some("base"));
	assert_eq!(steps[1].user.as_deref(), Some("parent"));
	assert_eq!(
		steps[1].command.as_ref().map(Command::to_exec),
		Some(vec!["second".into()])
	);
	assert!(!steps[1].environment.to_map().contains_key("EXTRA"));
}

#[test]
fn pre_start_does_not_recurse_or_restart() {
	let file = parse_str(
		r#"
services:
  web:
    image: base
    container_name: fixed
    restart: always
    deploy:
      restart_policy: {condition: any}
    post_start: [{command: post}]
    pre_stop: [{command: stop}]
    pre_start:
      - command: init
        pre_start: [{command: nested}]
"#,
	)
	.unwrap();
	let step = build_steps(&file.services["web"]).remove(0);
	assert!(step.pre_start.is_empty());
	assert!(step.post_start.is_empty());
	assert!(step.pre_stop.is_empty());
	assert!(step.container_name.is_none());
	assert!(matches!(
		step.restart,
		Some(crate::compose::types::RestartPolicy::No)
	));
	assert!(step.deploy.unwrap().restart_policy.is_none());
}
