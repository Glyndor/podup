//! Parse tests for features present in the type system but not previously covered.
use podup::compose::types::*;
use podup::parse_str;

// ---------------------------------------------------------------------------
// configs: content and environment
// ---------------------------------------------------------------------------

#[test]
fn config_content_multiline() {
	let yaml = r#"
configs:
  app_conf:
    content: |
      [server]
      port = 8080
"#;
	let content = parse_str(yaml).unwrap().configs["app_conf"]
		.content
		.clone()
		.unwrap();
	assert!(content.contains("port = 8080"));
}

#[test]
fn config_from_environment() {
	let yaml = r#"
configs:
  token:
    environment: AUTH_TOKEN
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.configs["token"].environment.as_deref(),
		Some("AUTH_TOKEN")
	);
}

#[test]
fn config_long_form_uid_gid_mode() {
	let yaml = r#"
configs:
  app_cfg:
    file: ./app.conf
services:
  app:
    image: alpine
    configs:
      - source: app_cfg
        target: /etc/app.conf
        uid: "500"
        gid: "500"
        mode: 292
"#;
	let file = parse_str(yaml).unwrap();
	let cref = &file.services["app"].configs[0];
	assert_eq!(cref.source(), "app_cfg");
	assert_eq!(cref.target(), Some("/etc/app.conf"));
	match cref {
		ServiceConfigRef::Long { uid, gid, mode, .. } => {
			assert_eq!(uid.as_deref(), Some("500"));
			assert_eq!(gid.as_deref(), Some("500"));
			assert_eq!(*mode, Some(292));
		}
		_ => panic!("expected long-form config ref"),
	}
}

// ---------------------------------------------------------------------------
// IPAM network config
// ---------------------------------------------------------------------------

#[test]
fn network_ipam_subnet_gateway() {
	let yaml = r#"
networks:
  backend:
    driver: bridge
    ipam:
      driver: default
      config:
        - subnet: 192.168.90.0/24
          gateway: 192.168.90.1
          ip_range: 192.168.90.128/25
"#;
	let file = parse_str(yaml).unwrap();
	let net = file.networks["backend"].as_ref().unwrap();
	let ipam = net.ipam.as_ref().unwrap();
	assert_eq!(ipam.driver.as_deref(), Some("default"));
	assert_eq!(ipam.config.len(), 1);
	assert_eq!(ipam.config[0].subnet.as_deref(), Some("192.168.90.0/24"));
	assert_eq!(ipam.config[0].gateway.as_deref(), Some("192.168.90.1"));
	assert_eq!(
		ipam.config[0].ip_range.as_deref(),
		Some("192.168.90.128/25")
	);
}

#[test]
fn network_ipam_aux_addresses() {
	let yaml = r#"
networks:
  mynet:
    ipam:
      config:
        - subnet: 172.16.238.0/24
          aux_addresses:
            host1: 172.16.238.5
"#;
	let file = parse_str(yaml).unwrap();
	let net = file.networks["mynet"].as_ref().unwrap();
	let pool = &net.ipam.as_ref().unwrap().config[0];
	assert_eq!(
		pool.aux_addresses.get("host1").map(|s| s.as_str()),
		Some("172.16.238.5")
	);
}

// ---------------------------------------------------------------------------
// pids_limit (top-level service field)
// ---------------------------------------------------------------------------

#[test]
fn pids_limit_service_field() {
	let yaml = "services:\n  app:\n    image: alpine\n    pids_limit: 256\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].pids_limit,
		Some(256)
	);
}

// ---------------------------------------------------------------------------
// Deploy: restart_policy, update_config, rollback_config, placement
// ---------------------------------------------------------------------------

#[test]
fn deploy_restart_policy() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      restart_policy:
        condition: on-failure
        delay: 5s
        max_attempts: 3
        window: 120s
"#;
	let file = parse_str(yaml).unwrap();
	let rp = file.services["app"]
		.deploy
		.as_ref()
		.unwrap()
		.restart_policy
		.as_ref()
		.unwrap();
	assert_eq!(rp.condition.as_deref(), Some("on-failure"));
	assert_eq!(rp.max_attempts, Some(3));
}

#[test]
fn deploy_update_config() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      update_config:
        parallelism: 2
        delay: 10s
        failure_action: rollback
        order: start-first
"#;
	let file = parse_str(yaml).unwrap();
	let uc = file.services["app"]
		.deploy
		.as_ref()
		.unwrap()
		.update_config
		.as_ref()
		.unwrap();
	assert_eq!(uc.parallelism, Some(2));
	assert_eq!(uc.failure_action.as_deref(), Some("rollback"));
}

#[test]
fn deploy_rollback_config() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      rollback_config:
        parallelism: 1
        delay: 5s
"#;
	let file = parse_str(yaml).unwrap();
	let rc = file.services["app"]
		.deploy
		.as_ref()
		.unwrap()
		.rollback_config
		.as_ref()
		.unwrap();
	assert_eq!(rc.parallelism, Some(1));
}

#[test]
fn deploy_placement_constraints() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      placement:
        constraints:
          - "node.role==manager"
          - "node.labels.datacenter==east"
        max_replicas_per_node: 3
"#;
	let file = parse_str(yaml).unwrap();
	let pl = file.services["app"]
		.deploy
		.as_ref()
		.unwrap()
		.placement
		.as_ref()
		.unwrap();
	assert_eq!(pl.constraints.len(), 2);
	assert_eq!(pl.max_replicas_per_node, Some(3));
}

#[test]
fn deploy_mode_and_endpoint_mode() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      mode: replicated
      endpoint_mode: vip
"#;
	let file = parse_str(yaml).unwrap();
	let deploy = file.services["app"].deploy.as_ref().unwrap();
	assert_eq!(deploy.mode.as_deref(), Some("replicated"));
	assert_eq!(deploy.endpoint_mode.as_deref(), Some("vip"));
}

// ---------------------------------------------------------------------------
// Build: no_cache, pull, tags, privileged, extra_hosts, additional_contexts,
//        dockerfile_inline, ssh, secrets, ulimits
// ---------------------------------------------------------------------------

#[test]
fn build_no_cache_and_pull() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      no_cache: true
      pull: true
"#;
	let file = parse_str(yaml).unwrap();
	let build = file.services["app"].build.as_ref().unwrap();
	assert!(build.no_cache());
	assert!(build.pull());
}

#[test]
fn build_tags() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      tags:
        - myregistry.io/myapp:v1.2.3
        - myregistry.io/myapp:latest
"#;
	let file = parse_str(yaml).unwrap();
	let build = file.services["app"].build.as_ref().unwrap();
	assert_eq!(build.tags().len(), 2);
	assert!(build.tags()[0].contains("v1.2.3"));
}

#[test]
fn build_privileged() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      privileged: true
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config { privileged, .. } => assert_eq!(*privileged, Some(true)),
		_ => panic!("expected long-form build"),
	}
}

#[test]
fn build_extra_hosts() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      extra_hosts:
        - "somehost:162.242.195.82"
"#;
	let file = parse_str(yaml).unwrap();
	let build = file.services["app"].build.as_ref().unwrap();
	assert_eq!(build.extra_hosts().len(), 1);
}

#[test]
fn build_additional_contexts() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      additional_contexts:
        mylib: /path/to/mylib
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config {
			additional_contexts,
			..
		} => {
			assert_eq!(
				additional_contexts.get("mylib").map(|s| s.as_str()),
				Some("/path/to/mylib")
			);
		}
		_ => panic!("expected long-form build"),
	}
}

#[test]
fn build_dockerfile_inline() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      dockerfile_inline: |
        FROM alpine
        RUN echo hello
"#;
	let file = parse_str(yaml).unwrap();
	let build = file.services["app"].build.as_ref().unwrap();
	let inline = build.dockerfile_inline().unwrap();
	assert!(inline.contains("FROM alpine"));
}

#[test]
fn build_ssh_and_secrets() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      ssh:
        - default
      secrets:
        - server-certificate
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config { ssh, secrets, .. } => {
			assert_eq!(ssh.len(), 1);
			assert_eq!(secrets.len(), 1);
			assert_eq!(secrets[0], "server-certificate");
		}
		_ => panic!("expected long-form build"),
	}
}
