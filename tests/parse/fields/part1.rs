//! Field-level parse tests (split into part files for the line limit).
use podup::compose::types::*;
use podup::parse_str;

#[test]
fn sysctls_as_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    sysctls:
      - net.core.somaxconn=1024
      - net.ipv4.ip_forward=1
"#;
	let sc = parse_str(yaml).unwrap().services["app"].sysctls.to_map();
	assert_eq!(
		sc.get("net.core.somaxconn").map(|s| s.as_str()),
		Some("1024")
	);
	assert_eq!(sc.get("net.ipv4.ip_forward").map(|s| s.as_str()), Some("1"));
}

#[test]
fn sysctls_as_map() {
	let yaml = r#"
services:
  app:
    image: alpine
    sysctls:
      net.core.somaxconn: "1024"
"#;
	let sc = parse_str(yaml).unwrap().services["app"].sysctls.to_map();
	assert_eq!(
		sc.get("net.core.somaxconn").map(|s| s.as_str()),
		Some("1024")
	);
}

#[test]
fn sysctls_as_map_with_int() {
	let yaml = r#"
services:
  app:
    image: alpine
    sysctls:
      net.ipv4.ip_forward: 1
"#;
	let sc = parse_str(yaml).unwrap().services["app"].sysctls.to_map();
	assert_eq!(sc.get("net.ipv4.ip_forward").map(|s| s.as_str()), Some("1"));
}

#[test]
fn ulimits_as_number() {
	let yaml = r#"
services:
  app:
    image: alpine
    ulimits:
      nofile: 1024
"#;
	let ul = parse_str(yaml).unwrap();
	let ul = &ul.services["app"].ulimits["nofile"];
	assert_eq!(ul.soft(), 1024);
	assert_eq!(ul.hard(), 1024);
}

#[test]
fn ulimits_as_object() {
	let yaml = r#"
services:
  app:
    image: alpine
    ulimits:
      nofile:
        soft: 1024
        hard: 65536
"#;
	let file = parse_str(yaml).unwrap();
	let ul = &file.services["app"].ulimits["nofile"];
	assert_eq!(ul.soft(), 1024);
	assert_eq!(ul.hard(), 65536);
}

#[test]
fn logging_config() {
	let yaml = r#"
services:
  app:
    image: alpine
    logging:
      driver: json-file
      options:
        max-size: 10m
        max-file: "3"
"#;
	let file = parse_str(yaml).unwrap();
	let logging = file.services["app"].logging.as_ref().unwrap();
	assert_eq!(logging.driver.as_deref(), Some("json-file"));
	assert_eq!(
		logging.options.get("max-size").map(|s| s.as_str()),
		Some("10m")
	);
}

#[test]
fn deploy_config() {
	let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      replicas: 3
      resources:
        limits:
          cpus: "0.5"
          memory: 128M
"#;
	let file = parse_str(yaml).unwrap();
	let deploy = file.services["app"].deploy.as_ref().unwrap();
	assert_eq!(deploy.replicas, Some(3));
	let limits = deploy.resources.as_ref().unwrap().limits.as_ref().unwrap();
	assert_eq!(limits.cpus.as_deref(), Some("0.5"));
	assert_eq!(limits.memory.as_deref(), Some("128M"));
}

#[test]
fn network_mode_host() {
	let yaml = "services:\n  app:\n    image: alpine\n    network_mode: host\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.network_mode
			.as_deref(),
		Some("host")
	);
}

#[test]
fn profiles() {
	let yaml = "services:\n  debug:\n    image: alpine\n    profiles: [debug, dev]\n";
	let profiles = &parse_str(yaml).unwrap().services["debug"].profiles;
	assert!(profiles.contains(&"debug".to_string()));
	assert!(profiles.contains(&"dev".to_string()));
}

#[test]
fn secrets_on_service() {
	let yaml = r#"
services:
  app:
    image: alpine
    secrets: [my_secret, ext_secret]
secrets:
  my_secret:
    file: ./secret.txt
  ext_secret:
    external: true
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].secrets.len(), 2);
	assert_eq!(file.services["app"].secrets[0].source(), "my_secret");
	assert_eq!(
		file.secrets["my_secret"].file.as_deref(),
		Some("./secret.txt")
	);
}

#[test]
fn env_file_string() {
	let yaml = "services:\n  app:\n    image: alpine\n    env_file: .env\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].env_file.to_list(),
		vec![".env"]
	);
}

#[test]
fn env_file_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    env_file: [.env, .env.local]
"#;
	let list = parse_str(yaml).unwrap().services["app"].env_file.to_list();
	assert_eq!(list.len(), 2);
	assert!(list.contains(&".env.local".to_string()));
}

#[test]
fn restart_policies() {
	for policy in &["no", "always", "on-failure", "unless-stopped"] {
		let yaml = format!("services:\n  app:\n    image: alpine\n    restart: {policy}\n");
		assert!(parse_str(&yaml).unwrap().services["app"].restart.is_some());
	}
}

#[test]
fn restart_on_failure_with_count() {
	let yaml = "services:\n  app:\n    image: alpine\n    restart: on-failure:5\n";
	let r = parse_str(yaml).unwrap().services["app"]
		.restart
		.clone()
		.unwrap();
	match r {
		RestartPolicy::OnFailure { max_attempts } => assert_eq!(max_attempts, Some(5)),
		other => panic!("expected OnFailure, got {other:?}"),
	}
}

#[test]
fn labels_as_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    labels:
      - "com.example.env=prod"
"#;
	let labels = parse_str(yaml).unwrap().services["app"].labels.to_map();
	assert_eq!(
		labels.get("com.example.env").map(|s| s.as_str()),
		Some("prod")
	);
}

#[test]
fn labels_as_map() {
	let yaml = "services:\n  app:\n    image: alpine\n    labels:\n      com.example.env: prod\n";
	let labels = parse_str(yaml).unwrap().services["app"].labels.to_map();
	assert_eq!(
		labels.get("com.example.env").map(|s| s.as_str()),
		Some("prod")
	);
}

#[test]
fn extra_hosts() {
	let yaml = r#"
services:
  app:
    image: alpine
    extra_hosts:
      - "somehost:162.242.195.82"
      - "otherhost:50.31.209.229"
"#;
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].extra_hosts.len(),
		2
	);
}

#[test]
fn extra_hosts_mapping_form_normalizes_to_host_colon_ip() {
	// Compose also allows the mapping form; it must normalize to "host:ip".
	let yaml = r#"
services:
  app:
    image: alpine
    extra_hosts:
      somehost: "162.242.195.82"
      otherhost: "50.31.209.229"
"#;
	let hosts = &parse_str(yaml).unwrap().services["app"].extra_hosts;
	assert!(hosts.contains(&"somehost:162.242.195.82".to_string()));
	assert!(hosts.contains(&"otherhost:50.31.209.229".to_string()));
}

#[test]
fn tty_and_stdin_open() {
	let yaml = "services:\n  app:\n    image: alpine\n    tty: true\n    stdin_open: true\n";
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].tty, Some(true));
	assert_eq!(file.services["app"].stdin_open, Some(true));
}

#[test]
fn privileged_and_init() {
	let yaml = "services:\n  app:\n    image: alpine\n    privileged: true\n    init: true\n";
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].privileged, Some(true));
	assert_eq!(file.services["app"].init, Some(true));
}

#[test]
fn stop_signal() {
	let yaml = "services:\n  app:\n    image: alpine\n    stop_signal: SIGTERM\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.stop_signal
			.as_deref(),
		Some("SIGTERM")
	);
}

#[test]
fn dns() {
	let yaml = r#"
services:
  app:
    image: alpine
    dns: [8.8.8.8, 8.8.4.4]
"#;
	assert!(parse_str(yaml).unwrap().services["app"]
		.dns
		.to_list()
		.contains(&"8.8.8.8".to_string()));
}

#[test]
fn cap_add_and_drop() {
	let yaml =
		"services:\n  app:\n    image: alpine\n    cap_add: [NET_ADMIN]\n    cap_drop: [ALL]\n";
	let file = parse_str(yaml).unwrap();
	assert!(file.services["app"]
		.cap_add
		.contains(&"NET_ADMIN".to_string()));
	assert!(file.services["app"].cap_drop.contains(&"ALL".to_string()));
}
