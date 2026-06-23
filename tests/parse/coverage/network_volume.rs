//! Parse tests for features present in the type system but not previously covered.
use podup::compose::types::*;
use podup::parse_str;

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
