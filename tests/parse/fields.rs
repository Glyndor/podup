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

// ---------------------------------------------------------------------------
// New: extends, configs, annotations, scale, complex volumes, build extras,
// network maps, healthcheck disable, podman network modes
// ---------------------------------------------------------------------------

#[test]
fn extends_short_form() {
    let yaml = r#"
services:
  base:
    image: alpine
    environment:
      LOG: info
  app:
    extends: base
"#;
    let file = parse_str(yaml).unwrap();
    let app = &file.services["app"];
    assert_eq!(app.image.as_deref(), Some("alpine"));
    let env = app.environment.to_map();
    assert_eq!(
        env.get("LOG").and_then(|v| v.clone()).as_deref(),
        Some("info")
    );
}

#[test]
fn extends_long_form_no_file() {
    let yaml = r#"
services:
  base:
    image: alpine
  app:
    extends:
      service: base
"#;
    let file = parse_str(yaml).unwrap();
    assert_eq!(file.services["app"].image.as_deref(), Some("alpine"));
}

#[test]
fn extends_with_file_field_parses() {
    // Just verify that extends with file is parsed (resolution requires parse_file).
    let yaml = r#"
services:
  app:
    image: alpine
    extends:
      service: base
      file: ./common.yml
"#;
    // parse_str does not resolve external files; the field should still parse,
    // but extends must remain unresolved (we expect an error from parse_str).
    let res = parse_str(yaml);
    assert!(
        res.is_err(),
        "parse_str should reject extends.file references"
    );
}

#[test]
fn configs_top_level() {
    let yaml = r#"
configs:
  app_cfg:
    file: ./app.conf
  inline_cfg:
    content: "hello world"
services:
  app:
    image: alpine
    configs:
      - app_cfg
      - source: inline_cfg
        target: /etc/inline.conf
        mode: 420
"#;
    let file = parse_str(yaml).unwrap();
    assert!(file.configs.contains_key("app_cfg"));
    assert_eq!(file.configs["app_cfg"].file.as_deref(), Some("./app.conf"));
    assert_eq!(
        file.configs["inline_cfg"].content.as_deref(),
        Some("hello world")
    );
    assert_eq!(file.services["app"].configs.len(), 2);
    assert_eq!(file.services["app"].configs[0].source(), "app_cfg");
    assert_eq!(
        file.services["app"].configs[1].target(),
        Some("/etc/inline.conf")
    );
}

#[test]
fn annotations_as_list_and_map() {
    let yaml_list = r#"
services:
  a:
    image: alpine
    annotations:
      - "io.k8s.example=foo"
"#;
    let yaml_map = r#"
services:
  a:
    image: alpine
    annotations:
      io.k8s.example: foo
"#;
    for yaml in &[yaml_list, yaml_map] {
        let m = parse_str(yaml).unwrap().services["a"].annotations.to_map();
        assert_eq!(m.get("io.k8s.example").map(|s| s.as_str()), Some("foo"));
    }
}

#[test]
fn scale_field() {
    let yaml = "services:\n  app:\n    image: alpine\n    scale: 3\n";
    assert_eq!(parse_str(yaml).unwrap().services["app"].scale, Some(3));
}

#[test]
fn volume_long_form_bind_propagation() {
    let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: bind
        source: /host
        target: /cont
        bind:
          propagation: rshared
          create_host_path: true
          selinux: z
"#;
    let file = parse_str(yaml).unwrap();
    let v = &file.services["app"].volumes[0];
    match v {
        VolumeMount::Long { bind: Some(b), .. } => {
            assert_eq!(b.propagation.as_deref(), Some("rshared"));
            assert_eq!(b.create_host_path, Some(true));
            assert_eq!(b.selinux.as_deref(), Some("z"));
        }
        _ => panic!("expected long-form bind mount"),
    }
}

#[test]
fn volume_long_form_tmpfs_size_mode() {
    let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: tmpfs
        target: /run
        tmpfs:
          size: 67108864
          mode: 1023
"#;
    let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
    match v {
        VolumeMount::Long { tmpfs: Some(t), .. } => {
            assert_eq!(t.size, Some(67108864));
            assert_eq!(t.mode, Some(1023));
        }
        _ => panic!("expected tmpfs mount"),
    }
}

#[test]
fn volume_long_form_volume_nocopy() {
    let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: volume
        source: data
        target: /data
        volume:
          nocopy: true
"#;
    let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
    match v {
        VolumeMount::Long {
            volume: Some(vo), ..
        } => assert_eq!(vo.nocopy, Some(true)),
        _ => panic!("expected long-form volume mount"),
    }
}

#[test]
fn build_cache_from() {
    let yaml = r#"
services:
  app:
    build:
      context: .
      cache_from:
        - registry/example:latest
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    match svc.build.as_ref().unwrap() {
        BuildConfig::Config { cache_from, .. } => assert_eq!(cache_from.len(), 1),
        _ => panic!("expected long-form build"),
    }
}

#[test]
fn build_shm_size() {
    let yaml = r#"
services:
  app:
    build:
      context: .
      shm_size: 128m
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    match svc.build.as_ref().unwrap() {
        BuildConfig::Config { shm_size, .. } => assert_eq!(shm_size.as_deref(), Some("128m")),
        _ => panic!(""),
    }
}

#[test]
fn build_network() {
    let yaml = r#"
services:
  app:
    build:
      context: .
      network: host
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    match svc.build.as_ref().unwrap() {
        BuildConfig::Config { network, .. } => assert_eq!(network.as_deref(), Some("host")),
        _ => panic!(""),
    }
}

#[test]
fn build_platforms() {
    let yaml = r#"
services:
  app:
    build:
      context: .
      platforms:
        - linux/amd64
        - linux/arm64
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    match svc.build.as_ref().unwrap() {
        BuildConfig::Config { platforms, .. } => assert_eq!(platforms.len(), 2),
        _ => panic!(""),
    }
}

#[test]
fn network_per_service_map_form() {
    let yaml = r#"
networks:
  frontend:
  backend:
services:
  app:
    image: alpine
    networks:
      frontend:
        aliases: [web, www]
        ipv4_address: 172.16.238.10
        ipv6_address: 2001:db8::10
      backend: ~
"#;
    let file = parse_str(yaml).unwrap();
    let names = file.services["app"].networks.names();
    assert!(names.contains(&"frontend".to_string()));
    assert!(names.contains(&"backend".to_string()));
    let cfg = file.services["app"]
        .networks
        .config_for("frontend")
        .expect("frontend cfg");
    assert_eq!(cfg.aliases.as_ref().unwrap().len(), 2);
    assert_eq!(cfg.ipv4_address.as_deref(), Some("172.16.238.10"));
    assert_eq!(cfg.ipv6_address.as_deref(), Some("2001:db8::10"));
}

#[test]
fn healthcheck_disable_via_test_none() {
    let yaml = r#"
services:
  app:
    image: alpine
    healthcheck:
      test: ["NONE"]
"#;
    let hc = parse_str(yaml).unwrap().services["app"]
        .healthcheck
        .as_ref()
        .unwrap()
        .clone();
    assert!(hc.is_disabled());
}

#[test]
fn healthcheck_disable_explicit() {
    let yaml = r#"
services:
  app:
    image: alpine
    healthcheck:
      disable: true
"#;
    let hc = parse_str(yaml).unwrap().services["app"]
        .healthcheck
        .as_ref()
        .unwrap()
        .clone();
    assert!(hc.is_disabled());
}

#[test]
fn healthcheck_start_interval() {
    let yaml = r#"
services:
  app:
    image: alpine
    healthcheck:
      test: ["CMD", "true"]
      start_interval: 1s
"#;
    let hc = parse_str(yaml).unwrap().services["app"]
        .healthcheck
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(hc.start_interval.as_deref(), Some("1s"));
}

#[test]
fn network_mode_slirp4netns() {
    let yaml = "services:\n  app:\n    image: alpine\n    network_mode: slirp4netns\n";
    assert_eq!(
        parse_str(yaml).unwrap().services["app"]
            .network_mode
            .as_deref(),
        Some("slirp4netns")
    );
}

#[test]
fn network_mode_pasta() {
    let yaml = "services:\n  app:\n    image: alpine\n    network_mode: pasta\n";
    assert_eq!(
        parse_str(yaml).unwrap().services["app"]
            .network_mode
            .as_deref(),
        Some("pasta")
    );
}

#[test]
fn network_mode_ns_path() {
    let yaml =
        "services:\n  app:\n    image: alpine\n    network_mode: \"ns:/run/netns/mynamespace\"\n";
    assert_eq!(
        parse_str(yaml).unwrap().services["app"]
            .network_mode
            .as_deref(),
        Some("ns:/run/netns/mynamespace")
    );
}
