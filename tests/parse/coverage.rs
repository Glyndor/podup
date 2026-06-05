/// Parse tests for features present in the type system but not previously covered.
use podup::compose::types::*;
use podup::parse_str;

// ---------------------------------------------------------------------------
// blkio_config
// ---------------------------------------------------------------------------

#[test]
fn blkio_weight() {
    let yaml = r#"
services:
  app:
    image: alpine
    blkio_config:
      weight: 300
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let bc = svc.blkio_config.as_ref().unwrap();
    assert_eq!(bc.weight, Some(300));
}

#[test]
fn blkio_weight_device() {
    let yaml = r#"
services:
  app:
    image: alpine
    blkio_config:
      weight_device:
        - path: /dev/sda
          weight: 400
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let bc = svc.blkio_config.as_ref().unwrap();
    assert_eq!(bc.weight_device.len(), 1);
    assert_eq!(bc.weight_device[0].path, "/dev/sda");
    assert_eq!(bc.weight_device[0].weight, 400);
}

#[test]
fn blkio_device_read_bps() {
    let yaml = r#"
services:
  app:
    image: alpine
    blkio_config:
      device_read_bps:
        - path: /dev/sda
          rate: "12mb"
      device_write_bps:
        - path: /dev/sda
          rate: "1024k"
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let bc = svc.blkio_config.as_ref().unwrap();
    assert_eq!(bc.device_read_bps.len(), 1);
    assert_eq!(bc.device_read_bps[0].path, "/dev/sda");
    assert_eq!(bc.device_write_bps.len(), 1);
}

#[test]
fn blkio_device_read_write_iops() {
    let yaml = r#"
services:
  app:
    image: alpine
    blkio_config:
      device_read_iops:
        - path: /dev/sda
          rate: 100
      device_write_iops:
        - path: /dev/sda
          rate: 200
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let bc = svc.blkio_config.as_ref().unwrap();
    assert_eq!(bc.device_read_iops.len(), 1);
    assert_eq!(bc.device_write_iops.len(), 1);
    assert_eq!(bc.device_read_iops[0].rate_value(), 100);
    assert_eq!(bc.device_write_iops[0].rate_value(), 200);
}

// ---------------------------------------------------------------------------
// gpus shorthand
// ---------------------------------------------------------------------------

#[test]
fn gpus_all() {
    let yaml = r#"
services:
  app:
    image: alpine
    gpus: all
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let gpus = svc.gpus.as_ref().unwrap();
    assert_eq!(gpus.to_count(), -1);
}

#[test]
fn gpus_count() {
    let yaml = r#"
services:
  app:
    image: alpine
    gpus: 2
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let gpus = svc.gpus.as_ref().unwrap();
    assert_eq!(gpus.to_count(), 2);
}

// ---------------------------------------------------------------------------
// deploy.resources.reservations.devices (GPU reservation)
// ---------------------------------------------------------------------------

#[test]
fn deploy_gpu_device_reservation() {
    let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      resources:
        reservations:
          devices:
            - capabilities: [gpu]
              count: 1
            - capabilities: [gpu]
              count: all
              device_ids: ["GPU-0", "GPU-1"]
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let deploy = svc.deploy.as_ref().unwrap();
    let devs = &deploy
        .resources
        .as_ref()
        .unwrap()
        .reservations
        .as_ref()
        .unwrap()
        .devices;
    assert_eq!(devs.len(), 2);
    assert!(devs[0].capabilities.contains(&"gpu".to_string()));
    assert_eq!(devs[1].device_ids.len(), 2);
}

// ---------------------------------------------------------------------------
// develop.watch
// ---------------------------------------------------------------------------

#[test]
fn develop_watch_sync() {
    let yaml = r#"
services:
  app:
    image: alpine
    develop:
      watch:
        - path: ./src
          action: sync
          target: /app/src
          ignore:
            - node_modules
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let dev = svc.develop.as_ref().unwrap();
    assert_eq!(dev.watch.len(), 1);
    let rule = &dev.watch[0];
    assert_eq!(rule.path, "./src");
    assert_eq!(rule.action, WatchAction::Sync);
    assert_eq!(rule.target.as_deref(), Some("/app/src"));
    assert_eq!(rule.ignore.len(), 1);
}

#[test]
fn develop_watch_rebuild() {
    let yaml = r#"
services:
  app:
    image: alpine
    develop:
      watch:
        - path: ./Dockerfile
          action: rebuild
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    let rule = &svc.develop.as_ref().unwrap().watch[0];
    assert_eq!(rule.action, WatchAction::Rebuild);
}

#[test]
fn develop_watch_sync_restart() {
    let yaml = r#"
services:
  app:
    image: alpine
    develop:
      watch:
        - path: ./config
          action: sync+restart
          target: /etc/app
          initial_sync: true
"#;
    let file = parse_str(yaml).unwrap();
    let rule = &file.services["app"].develop.as_ref().unwrap().watch[0];
    assert_eq!(rule.action, WatchAction::SyncAndRestart);
    assert!(rule.initial_sync);
}

// ---------------------------------------------------------------------------
// post_start / pre_stop lifecycle hooks
// ---------------------------------------------------------------------------

#[test]
fn post_start_exec_list() {
    let yaml = r#"
services:
  app:
    image: alpine
    post_start:
      - command: ["/scripts/init.sh", "--quiet"]
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    assert_eq!(svc.post_start.len(), 1);
    let cmd = svc.post_start[0].command.to_exec();
    assert_eq!(cmd[0], "/scripts/init.sh");
    assert_eq!(cmd[1], "--quiet");
}

#[test]
fn pre_stop_with_env_and_user() {
    let yaml = r#"
services:
  app:
    image: alpine
    pre_stop:
      - command: ["/scripts/cleanup.sh"]
        user: "1000"
        privileged: false
        working_dir: /app
        environment:
          CLEANUP: "true"
"#;
    let hook = &parse_str(yaml).unwrap().services["app"].pre_stop[0];
    assert_eq!(hook.user.as_deref(), Some("1000"));
    assert_eq!(hook.privileged, Some(false));
    assert_eq!(hook.working_dir.as_deref(), Some("/app"));
    let env = hook.environment.to_map();
    assert_eq!(
        env.get("CLEANUP").and_then(|v| v.clone()).as_deref(),
        Some("true")
    );
}

// ---------------------------------------------------------------------------
// env_file long-form
// ---------------------------------------------------------------------------

#[test]
fn env_file_long_form_with_required() {
    let yaml = r#"
services:
  app:
    image: alpine
    env_file:
      - path: .env.prod
        required: false
      - path: .env.local
        required: true
"#;
    let entries = parse_str(yaml).unwrap().services["app"]
        .env_file
        .to_entries();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].path(), ".env.prod");
    assert!(!entries[0].required());
    assert_eq!(entries[1].path(), ".env.local");
    assert!(entries[1].required());
}

#[test]
fn env_file_long_form_with_format() {
    let yaml = r#"
services:
  app:
    image: alpine
    env_file:
      - path: .env
        format: dotenv
"#;
    let entries = parse_str(yaml).unwrap().services["app"]
        .env_file
        .to_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path(), ".env");
}

#[test]
fn env_file_mixed_short_and_long() {
    let yaml = r#"
services:
  app:
    image: alpine
    env_file:
      - .env
      - path: .env.local
        required: false
"#;
    let entries = parse_str(yaml).unwrap().services["app"]
        .env_file
        .to_entries();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].required()); // short form defaults to required
    assert!(!entries[1].required());
}

// ---------------------------------------------------------------------------
// secrets top-level: content and environment
// ---------------------------------------------------------------------------

#[test]
fn secret_content_inline() {
    let yaml = r#"
secrets:
  db_password:
    content: "s3cr3t_password"
"#;
    let file = parse_str(yaml).unwrap();
    assert_eq!(
        file.secrets["db_password"].content.as_deref(),
        Some("s3cr3t_password")
    );
}

#[test]
fn secret_from_environment() {
    let yaml = r#"
secrets:
  api_key:
    environment: MY_API_KEY
"#;
    let file = parse_str(yaml).unwrap();
    assert_eq!(
        file.secrets["api_key"].environment.as_deref(),
        Some("MY_API_KEY")
    );
}

#[test]
fn secret_with_driver_and_labels() {
    let yaml = r#"
secrets:
  db_pass:
    driver: vault
    driver_opts:
      path: secret/db
    labels:
      team: backend
"#;
    let s = &parse_str(yaml).unwrap().secrets["db_pass"];
    assert_eq!(s.driver.as_deref(), Some("vault"));
    assert_eq!(
        s.driver_opts.get("path").map(|s| s.as_str()),
        Some("secret/db")
    );
    assert!(!s.labels.to_map().is_empty());
}

#[test]
fn secret_long_form_uid_gid_mode() {
    let yaml = r#"
secrets:
  app_cert:
    file: ./cert.pem
services:
  app:
    image: alpine
    secrets:
      - source: app_cert
        target: /run/secrets/cert.pem
        uid: "1000"
        gid: "1000"
        mode: 256
"#;
    let file = parse_str(yaml).unwrap();
    let sref = &file.services["app"].secrets[0];
    assert_eq!(sref.source(), "app_cert");
    assert_eq!(sref.target(), Some("/run/secrets/cert.pem"));
    match sref {
        ServiceSecretRef::Long { uid, gid, mode, .. } => {
            assert_eq!(uid.as_deref(), Some("1000"));
            assert_eq!(gid.as_deref(), Some("1000"));
            assert_eq!(*mode, Some(256));
        }
        _ => panic!("expected long-form secret ref"),
    }
}

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

// ---------------------------------------------------------------------------
// Network service config: gw_priority, mac_address, link_local_ips,
//                         interface_name
// ---------------------------------------------------------------------------

#[test]
fn network_service_gw_priority() {
    let yaml = r#"
networks:
  frontend:
services:
  app:
    image: alpine
    networks:
      frontend:
        gw_priority: 100
"#;
    let file = parse_str(yaml).unwrap();
    let cfg = file.services["app"]
        .networks
        .config_for("frontend")
        .unwrap();
    assert_eq!(cfg.gw_priority, Some(100));
}

#[test]
fn network_service_mac_address() {
    let yaml = r#"
networks:
  net:
services:
  app:
    image: alpine
    networks:
      net:
        mac_address: "02:42:ac:11:00:02"
"#;
    let file = parse_str(yaml).unwrap();
    let cfg = file.services["app"].networks.config_for("net").unwrap();
    assert_eq!(cfg.mac_address.as_deref(), Some("02:42:ac:11:00:02"));
}

#[test]
fn network_service_link_local_ips() {
    let yaml = r#"
networks:
  net:
services:
  app:
    image: alpine
    networks:
      net:
        link_local_ips:
          - 169.254.8.1
"#;
    let file = parse_str(yaml).unwrap();
    let cfg = file.services["app"].networks.config_for("net").unwrap();
    assert_eq!(cfg.link_local_ips.len(), 1);
    assert_eq!(cfg.link_local_ips[0], "169.254.8.1");
}

#[test]
fn network_service_interface_name() {
    let yaml = r#"
networks:
  net:
services:
  app:
    image: alpine
    networks:
      net:
        interface_name: eth0
"#;
    let file = parse_str(yaml).unwrap();
    let cfg = file.services["app"].networks.config_for("net").unwrap();
    assert_eq!(cfg.interface_name.as_deref(), Some("eth0"));
}

// ---------------------------------------------------------------------------
// Volume: consistency, driver_config, subpath, labels
// ---------------------------------------------------------------------------

#[test]
fn volume_consistency() {
    let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: volume
        source: data
        target: /data
        consistency: cached
"#;
    let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
    match v {
        VolumeMount::Long { consistency, .. } => {
            assert_eq!(consistency.as_deref(), Some("cached"));
        }
        _ => panic!("expected long-form volume"),
    }
}

#[test]
fn volume_options_driver_config() {
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
          driver_config:
            name: nfs
            options:
              addr: "nfs.example.com"
"#;
    let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
    match v {
        VolumeMount::Long {
            volume: Some(vo), ..
        } => {
            let dc = vo.driver_config.as_ref().unwrap();
            assert_eq!(dc.name.as_deref(), Some("nfs"));
            assert_eq!(
                dc.options.get("addr").map(|s| s.as_str()),
                Some("nfs.example.com")
            );
        }
        _ => panic!("expected long-form volume with options"),
    }
}

#[test]
fn volume_options_subpath() {
    let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: volume
        source: data
        target: /data
        volume:
          subpath: subdir/nested
"#;
    let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
    match v {
        VolumeMount::Long {
            volume: Some(vo), ..
        } => {
            assert_eq!(vo.subpath.as_deref(), Some("subdir/nested"));
        }
        _ => panic!("expected long-form volume"),
    }
}

#[test]
fn volume_options_labels() {
    let yaml = r#"
services:
  app:
    image: alpine
    volumes:
      - type: volume
        source: data
        target: /data
        volume:
          labels:
            backup: daily
"#;
    let v = &parse_str(yaml).unwrap().services["app"].volumes[0];
    match v {
        VolumeMount::Long {
            volume: Some(vo), ..
        } => {
            let labels = vo.labels.to_map();
            assert_eq!(labels.get("backup").map(|s| s.as_str()), Some("daily"));
        }
        _ => panic!("expected long-form volume"),
    }
}

// ---------------------------------------------------------------------------
// CPU realtime / cpu_count / cpu_percent fields
// ---------------------------------------------------------------------------

#[test]
fn cpu_count_and_percent() {
    let yaml = r#"
services:
  app:
    image: alpine
    cpu_count: 4
    cpu_percent: 75
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    assert_eq!(svc.cpu_count, Some(4));
    assert_eq!(svc.cpu_percent, Some(75));
}

#[test]
fn cpu_rt_runtime_and_period() {
    let yaml = r#"
services:
  app:
    image: alpine
    cpu_rt_runtime: 950000
    cpu_rt_period: 1000000
"#;
    let svc = &parse_str(yaml).unwrap().services["app"];
    assert_eq!(svc.cpu_rt_runtime, Some(950000));
    assert_eq!(svc.cpu_rt_period, Some(1000000));
}

// ---------------------------------------------------------------------------
// label_file and attach
// ---------------------------------------------------------------------------

#[test]
fn label_file_single() {
    let yaml = "services:\n  app:\n    image: alpine\n    label_file: ./labels.properties\n";
    let list = parse_str(yaml).unwrap().services["app"]
        .label_file
        .to_list();
    assert_eq!(list, vec!["./labels.properties"]);
}

#[test]
fn label_file_list() {
    let yaml = r#"
services:
  app:
    image: alpine
    label_file:
      - ./labels.properties
      - ./extra.labels
"#;
    let list = parse_str(yaml).unwrap().services["app"]
        .label_file
        .to_list();
    assert_eq!(list.len(), 2);
}

#[test]
fn attach_field() {
    let yaml = "services:\n  app:\n    image: alpine\n    attach: false\n";
    assert_eq!(parse_str(yaml).unwrap().services["app"].attach, Some(false));
}

// ---------------------------------------------------------------------------
// uts, cgroup namespace
// ---------------------------------------------------------------------------

#[test]
fn uts_host() {
    let yaml = "services:\n  app:\n    image: alpine\n    uts: host\n";
    assert_eq!(
        parse_str(yaml).unwrap().services["app"].uts.as_deref(),
        Some("host")
    );
}

#[test]
fn cgroup_field() {
    let yaml = "services:\n  app:\n    image: alpine\n    cgroup: host\n";
    assert_eq!(
        parse_str(yaml).unwrap().services["app"].cgroup.as_deref(),
        Some("host")
    );
}

// ---------------------------------------------------------------------------
// Build: isolation, entitlements, provenance, sbom
// ---------------------------------------------------------------------------

#[test]
fn build_isolation() {
    let yaml = r#"
services:
  app:
    build:
      context: .
      isolation: hyperv
"#;
    match parse_str(yaml).unwrap().services["app"]
        .build
        .as_ref()
        .unwrap()
    {
        BuildConfig::Config { isolation, .. } => assert_eq!(isolation.as_deref(), Some("hyperv")),
        _ => panic!("expected long-form build"),
    }
}

#[test]
fn build_entitlements() {
    let yaml = r#"
services:
  app:
    build:
      context: .
      entitlements:
        - network.host
        - security.insecure
"#;
    match parse_str(yaml).unwrap().services["app"]
        .build
        .as_ref()
        .unwrap()
    {
        BuildConfig::Config { entitlements, .. } => {
            assert_eq!(entitlements.len(), 2);
            assert!(entitlements.contains(&"network.host".to_string()));
        }
        _ => panic!("expected long-form build"),
    }
}

#[test]
fn build_sbom() {
    let yaml = r#"
services:
  app:
    build:
      context: .
      sbom: true
"#;
    match parse_str(yaml).unwrap().services["app"]
        .build
        .as_ref()
        .unwrap()
    {
        BuildConfig::Config { sbom, .. } => assert_eq!(*sbom, Some(true)),
        _ => panic!("expected long-form build"),
    }
}

// ---------------------------------------------------------------------------
// Networks: ipam options, multiple pools
// ---------------------------------------------------------------------------

#[test]
fn network_ipam_options() {
    let yaml = r#"
networks:
  mynet:
    ipam:
      driver: custom
      options:
        foo: bar
"#;
    let file = parse_str(yaml).unwrap();
    let ipam = file.networks["mynet"]
        .as_ref()
        .unwrap()
        .ipam
        .as_ref()
        .unwrap();
    assert_eq!(ipam.driver.as_deref(), Some("custom"));
    assert_eq!(ipam.options.get("foo").map(|s| s.as_str()), Some("bar"));
}

// ---------------------------------------------------------------------------
// deploy.labels
// ---------------------------------------------------------------------------

#[test]
fn deploy_labels_as_list() {
    let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      labels:
        - "com.example.description=API service"
        - "com.example.tier=backend"
"#;
    let file = parse_str(yaml).unwrap();
    let deploy = file.services["app"].deploy.as_ref().unwrap();
    let labels = deploy.labels.to_map();
    assert_eq!(
        labels.get("com.example.description").map(|s| s.as_str()),
        Some("API service")
    );
}

#[test]
fn deploy_labels_as_map() {
    let yaml = r#"
services:
  app:
    image: alpine
    deploy:
      labels:
        app.version: "1.0"
"#;
    let file = parse_str(yaml).unwrap();
    let deploy = file.services["app"].deploy.as_ref().unwrap();
    let labels = deploy.labels.to_map();
    assert_eq!(labels.get("app.version").map(|s| s.as_str()), Some("1.0"));
}

// ---------------------------------------------------------------------------
// service.dns_search (coverage of StringOrList as list form)
// ---------------------------------------------------------------------------

#[test]
fn dns_search_list() {
    let yaml = r#"
services:
  app:
    image: alpine
    dns_search:
      - example.com
      - internal.local
"#;
    let list = parse_str(yaml).unwrap().services["app"]
        .dns_search
        .to_list();
    assert!(list.contains(&"example.com".to_string()));
}

// ---------------------------------------------------------------------------
// Devices: cgroup rules
// ---------------------------------------------------------------------------

#[test]
fn device_cgroup_rules() {
    let yaml = r#"
services:
  app:
    image: alpine
    device_cgroup_rules:
      - "c 1:3 mr"
      - "b 7:* rmw"
"#;
    let rules = &parse_str(yaml).unwrap().services["app"].device_cgroup_rules;
    assert_eq!(rules.len(), 2);
    assert!(rules[0].contains("c 1:3"));
}
