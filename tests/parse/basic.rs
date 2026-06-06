use podup::compose::types::*;
use podup::parse_str;

#[test]
fn minimal() {
	let file = parse_str("services:\n  web:\n    image: nginx:alpine").unwrap();
	assert!(file.services.contains_key("web"));
	assert_eq!(file.services["web"].image.as_deref(), Some("nginx:alpine"));
}

#[test]
fn empty_services() {
	assert!(parse_str("services: {}").unwrap().services.is_empty());
}

#[test]
fn invalid_yaml() {
	assert!(parse_str("services: [invalid: yaml: here").is_err());
}

#[test]
fn env_as_list() {
	let yaml = r#"
services:
  app:
    image: node:20
    environment:
      - NODE_ENV=production
      - PORT=3000
      - SECRET
"#;
	let env = parse_str(yaml).unwrap().services["app"]
		.environment
		.to_map();
	assert_eq!(env["NODE_ENV"].as_deref(), Some("production"));
	assert_eq!(env["PORT"].as_deref(), Some("3000"));
	assert!(env.contains_key("SECRET"));
}

#[test]
fn env_as_map() {
	let yaml = r#"
services:
  app:
    image: node:20
    environment:
      NODE_ENV: production
      PORT: 3000
"#;
	let env = parse_str(yaml).unwrap().services["app"]
		.environment
		.to_map();
	assert_eq!(env["NODE_ENV"].as_deref(), Some("production"));
	assert_eq!(env["PORT"].as_deref(), Some("3000"));
}

#[test]
fn command_shell() {
	let yaml = r#"
services:
  app:
    image: node:20
    command: "node server.js --port 3000"
"#;
	let exec = parse_str(yaml).unwrap().services["app"]
		.command
		.as_ref()
		.unwrap()
		.to_exec();
	assert_eq!(exec[0], "sh");
	assert_eq!(exec[1], "-c");
}

#[test]
fn command_exec() {
	let yaml = r#"
services:
  app:
    image: node:20
    command: ["node", "server.js"]
"#;
	let exec = parse_str(yaml).unwrap().services["app"]
		.command
		.as_ref()
		.unwrap()
		.to_exec();
	assert_eq!(exec, vec!["node", "server.js"]);
}

#[test]
fn entrypoint_shell() {
	let yaml = r#"
services:
  app:
    image: alpine
    entrypoint: "/usr/local/bin/init.sh --foo"
"#;
	let ep = parse_str(yaml).unwrap().services["app"]
		.entrypoint
		.as_ref()
		.unwrap()
		.to_exec();
	assert_eq!(ep[0], "sh");
	assert_eq!(ep[1], "-c");
	assert!(ep[2].contains("init.sh"));
}

#[test]
fn entrypoint_exec() {
	let yaml = r#"
services:
  app:
    image: alpine
    entrypoint: ["/bin/init", "--foo"]
"#;
	let ep = parse_str(yaml).unwrap().services["app"]
		.entrypoint
		.as_ref()
		.unwrap()
		.to_exec();
	assert_eq!(ep, vec!["/bin/init", "--foo"]);
}

#[test]
fn ports_short() {
	let yaml = r#"
services:
  web:
    image: nginx
    ports: ["80:80", "443:443"]
"#;
	assert_eq!(parse_str(yaml).unwrap().services["web"].ports.len(), 2);
}

#[test]
fn volumes_short() {
	let yaml = r#"
services:
  db:
    image: postgres:17
    volumes:
      - ./data:/var/lib/postgresql/data
      - pgdata:/var/lib/postgresql/data2
volumes:
  pgdata:
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["db"].volumes.len(), 2);
	assert!(file.volumes.contains_key("pgdata"));
}

#[test]
fn networks_list() {
	let yaml = r#"
services:
  web:
    image: nginx
    networks: [frontend]
networks:
  frontend:
    driver: bridge
"#;
	let file = parse_str(yaml).unwrap();
	assert!(file.networks.contains_key("frontend"));
	assert_eq!(file.services["web"].networks.names(), vec!["frontend"]);
}

#[test]
fn healthcheck() {
	let yaml = r#"
services:
  db:
    image: postgres:17
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 3s
      retries: 10
"#;
	let hc = parse_str(yaml).unwrap().services["db"]
		.healthcheck
		.as_ref()
		.unwrap()
		.retries;
	assert_eq!(hc, Some(10));
}

#[test]
fn depends_on_list() {
	let yaml = r#"
services:
  app:
    image: node:20
    depends_on: [db, redis]
  db:
    image: postgres:17
  redis:
    image: redis:8
"#;
	let deps = parse_str(yaml).unwrap().services["app"]
		.depends_on
		.service_names();
	assert!(deps.contains(&"db".to_string()));
	assert!(deps.contains(&"redis".to_string()));
}

#[test]
fn depends_on_map_with_condition() {
	let yaml = r#"
services:
  app:
    image: node:20
    depends_on:
      db:
        condition: service_healthy
  db:
    image: postgres:17
"#;
	let file = parse_str(yaml).unwrap();
	let condition = file.services["app"].depends_on.condition_for("db");
	assert!(matches!(condition, ServiceCondition::ServiceHealthy));
}

#[test]
fn secrets_top_level() {
	let yaml = r#"
secrets:
  db_password:
    file: ./secrets/db_password.txt
  jwt_secret:
    external: true
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.secrets["db_password"].file.as_deref(),
		Some("./secrets/db_password.txt")
	);
	assert_eq!(file.secrets["jwt_secret"].external, Some(true));
}

#[test]
fn hostname_and_domainname() {
	let yaml =
		"services:\n  app:\n    image: alpine\n    hostname: web1\n    domainname: example.com\n";
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].hostname.as_deref(), Some("web1"));
	assert_eq!(
		file.services["app"].domainname.as_deref(),
		Some("example.com")
	);
}

#[test]
fn mac_address() {
	let yaml = "services:\n  app:\n    image: alpine\n    mac_address: 02:42:ac:11:00:01\n";
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.services["app"].mac_address.as_deref(),
		Some("02:42:ac:11:00:01")
	);
}

#[test]
fn read_only_root() {
	let yaml = "services:\n  app:\n    image: alpine\n    read_only: true\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].read_only,
		Some(true)
	);
}

#[test]
fn expose_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    expose: ["80", "443"]
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].expose, vec!["80", "443"]);
}

#[test]
fn volumes_from_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    volumes_from:
      - service:ro
      - container:mycontainer
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].volumes_from.len(), 2);
}

#[test]
fn tmpfs_as_string() {
	let yaml = "services:\n  app:\n    image: alpine\n    tmpfs: /run\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].tmpfs.to_list(),
		vec!["/run"]
	);
}

#[test]
fn tmpfs_as_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    tmpfs:
      - /run
      - /tmp
"#;
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.tmpfs
			.to_list()
			.len(),
		2
	);
}

#[test]
fn security_opt_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    security_opt:
      - "no-new-privileges:true"
      - "label=disable"
"#;
	let so = &parse_str(yaml).unwrap().services["app"].security_opt;
	assert_eq!(so.len(), 2);
	assert!(so.iter().any(|s| s.contains("no-new-privileges")));
}

#[test]
fn devices_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    devices:
      - "/dev/sda:/dev/xvda:rwm"
      - "/dev/null:/dev/null"
"#;
	assert_eq!(parse_str(yaml).unwrap().services["app"].devices.len(), 2);
}

#[test]
fn group_add_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    group_add: [audio, video, "1000"]
"#;
	let g = &parse_str(yaml).unwrap().services["app"].group_add;
	assert_eq!(g.len(), 3);
	assert!(g.contains(&"audio".to_string()));
}

#[test]
fn userns_mode() {
	let yaml =
		"services:\n  app:\n    image: alpine\n    userns_mode: \"keep-id:uid=1000,gid=1000\"\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.userns_mode
			.as_deref(),
		Some("keep-id:uid=1000,gid=1000")
	);
}

#[test]
fn shm_size() {
	let yaml = "services:\n  app:\n    image: alpine\n    shm_size: 128m\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"].shm_size.as_deref(),
		Some("128m")
	);
}

#[test]
fn cgroup_parent_field() {
	let yaml = "services:\n  app:\n    image: alpine\n    cgroup_parent: my-cgroup\n";
	assert_eq!(
		parse_str(yaml).unwrap().services["app"]
			.cgroup_parent
			.as_deref(),
		Some("my-cgroup")
	);
}

#[test]
fn cpu_fields() {
	let yaml = r#"
services:
  app:
    image: alpine
    cpu_shares: 512
    cpuset: "0-1"
    mem_limit: 256m
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	assert_eq!(svc.cpu_shares, Some(512));
	assert_eq!(svc.cpuset.as_deref(), Some("0-1"));
	assert_eq!(svc.mem_limit.as_deref(), Some("256m"));
}
