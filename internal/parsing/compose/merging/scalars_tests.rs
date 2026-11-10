use super::super::merge::deserialize_with_merge;
use crate::compose::types::ComposeFile;

fn parse(yaml: &str) -> ComposeFile {
	deserialize_with_merge(yaml).expect("the file must parse")
}

#[test]
fn quoted_service_booleans_are_read_as_booleans() {
	let file = parse(
		r#"
services:
  a:
    image: busybox
    privileged: "true"
    read_only: "True"
    tty: "yes"
    init: "on"
    stdin_open: "y"
    oom_kill_disable: "false"
"#,
	);
	let a = &file.services["a"];
	assert_eq!(a.privileged, Some(true));
	assert_eq!(a.read_only, Some(true));
	assert_eq!(a.tty, Some(true));
	assert_eq!(a.init, Some(true));
	assert_eq!(a.stdin_open, Some(true));
	assert_eq!(a.oom_kill_disable, Some(false));
}

#[test]
fn quoted_service_integers_are_read_as_integers() {
	let file = parse(
		r#"
services:
  a:
    image: busybox
    pids_limit: "5"
    cpu_shares: "512"
    scale: "2"
    oom_score_adj: "-100"
    mem_swappiness: "+10"
"#,
	);
	let a = &file.services["a"];
	assert_eq!(a.pids_limit, Some(5));
	assert_eq!(a.cpu_shares, Some(512));
	assert_eq!(a.scale, Some(2));
	assert_eq!(a.oom_score_adj, Some(-100));
	assert_eq!(a.mem_swappiness, Some(10));
}

#[test]
fn quoted_scalars_inside_nested_blocks_are_read_as_their_type() {
	let file = parse(
		r#"
services:
  a:
    image: busybox
    healthcheck:
      disable: "true"
      retries: "3"
    depends_on:
      b:
        condition: service_started
        required: "false"
        restart: "true"
    deploy:
      replicas: "3"
      update_config:
        max_failure_ratio: "0.5"
    volumes:
      - type: bind
        source: /s
        target: /t
        read_only: "true"
        bind:
          create_host_path: "false"
  b:
    image: busybox
"#,
	);
	let a = &file.services["a"];
	let health = a.healthcheck.as_ref().unwrap();
	assert_eq!(health.disable, Some(true));
	assert_eq!(health.retries, Some(3));
	assert!(!a.depends_on.required_for("b"));
	assert!(a.depends_on.restart_for("b"));
	let deploy = a.deploy.as_ref().unwrap();
	assert_eq!(deploy.replicas, Some(3));
	assert_eq!(
		deploy.update_config.as_ref().unwrap().max_failure_ratio,
		Some(0.5)
	);
}

#[test]
fn quoted_resource_booleans_are_read_as_booleans() {
	let file = parse(
		r#"
networks:
  n: {external: "true", internal: "false", attachable: "yes", enable_ipv6: "off"}
volumes:
  v: {external: "true"}
secrets:
  s: {external: "TRUE"}
configs:
  c: {external: "true"}
"#,
	);
	let n = file.networks["n"].as_ref().unwrap();
	assert_eq!(n.external, Some(true));
	assert_eq!(n.internal, Some(false));
	assert_eq!(n.attachable, Some(true));
	assert_eq!(n.enable_ipv6, Some(false));
	assert_eq!(file.volumes["v"].as_ref().unwrap().external, Some(true));
	assert_eq!(file.secrets["s"].external, Some(true));
	assert_eq!(file.configs["c"].external, Some(true));
}

#[test]
fn text_that_spells_no_value_still_fails_the_type() {
	for field in [
		"read_only: \"maybe\"",
		"read_only: \"1\"",
		"read_only: \" true\"",
		"pids_limit: \"abc\"",
		"pids_limit: \"5.0\"",
		"pids_limit: \" 5\"",
		"pids_limit: \"0x10\"",
	] {
		let yaml = format!("services:\n  a:\n    image: busybox\n    {field}\n");
		assert!(deserialize_with_merge(&yaml).is_err(), "{field}");
	}
}

#[test]
fn free_text_that_shares_a_field_name_is_left_alone() {
	let file = parse(
		r#"
services:
  a:
    image: busybox
    environment:
      tty: "true"
      pids_limit: "5"
    labels:
      read_only: "true"
"#,
	);
	let a = &file.services["a"];
	assert_eq!(
		a.labels.to_map().get("read_only").map(String::as_str),
		Some("true")
	);
	let env = format!("{:?}", a.environment);
	assert!(env.contains("\"true\"") && env.contains("\"5\""), "{env}");
}

#[test]
fn unquoted_scalars_are_unchanged() {
	let file = parse("services:\n  a:\n    image: busybox\n    tty: true\n    pids_limit: 7\n");
	assert_eq!(file.services["a"].tty, Some(true));
	assert_eq!(file.services["a"].pids_limit, Some(7));
}
