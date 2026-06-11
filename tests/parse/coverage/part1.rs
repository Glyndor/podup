//! Parse tests for features present in the type system but not previously covered.
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
	assert_eq!(bc.device_read_bps[0].rate_value(), 12 * 1024 * 1024);
	assert_eq!(bc.device_write_bps.len(), 1);
	assert_eq!(bc.device_write_bps[0].rate_value(), 1024 * 1024);
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
