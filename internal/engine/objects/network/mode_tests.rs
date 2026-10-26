use super::*;
use crate::compose::types::{ComposeFile, Service};

fn empty_file() -> ComposeFile {
	ComposeFile::default()
}

#[test]
fn resolve_network_mode_explicit_mode() {
	let svc = Service {
		network_mode: Some("host".to_string()),
		..Default::default()
	};
	let file = empty_file();
	let (ns, nets) = resolve_network_mode("web", &svc, &file, "proj");
	assert!(ns.is_some());
	assert_eq!(ns.unwrap().nsmode, "host");
	assert!(nets.is_empty());
}

fn file_with_service(svc_name: &str, svc: Service) -> ComposeFile {
	let mut file = empty_file();
	file.services.insert(svc_name.to_string(), svc);
	file
}

#[test]
fn network_mode_service_single_replica_uses_first_replica_name() {
	// Auto-generated container names are always index-suffixed, so even a
	// single-replica `network_mode: service:` target attaches to `-1`.
	let target = Service::default();
	let file = file_with_service("db", target);
	let svc = Service {
		network_mode: Some("service:db".to_string()),
		..Default::default()
	};
	let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
	let ns = ns.unwrap();
	assert_eq!(ns.nsmode, "container");
	assert_eq!(ns.value.as_deref(), Some("proj-db-1"));
}

#[test]
fn network_mode_service_scaled_replicas_resolves_replica_one() {
	// `scale:`/`deploy.replicas` > 1 means the base name does not exist,
	// docker-compose attaches to replica `-1`.
	let target = Service {
		scale: Some(3),
		..Default::default()
	};
	let file = file_with_service("db", target);
	let svc = Service {
		network_mode: Some("service:db".to_string()),
		..Default::default()
	};
	let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
	assert_eq!(ns.unwrap().value.as_deref(), Some("proj-db-1"));
}

#[test]
fn network_mode_service_deploy_replicas_resolves_replica_one() {
	use crate::compose::types::DeployConfig;
	let target = Service {
		deploy: Some(DeployConfig {
			replicas: Some(2),
			..Default::default()
		}),
		..Default::default()
	};
	let file = file_with_service("db", target);
	let svc = Service {
		network_mode: Some("service:db".to_string()),
		..Default::default()
	};
	let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
	assert_eq!(ns.unwrap().value.as_deref(), Some("proj-db-1"));
}

#[test]
fn network_mode_service_container_name_wins_over_replica() {
	// An explicit container_name is honoured verbatim even when scaled.
	let target = Service {
		scale: Some(4),
		container_name: Some("custom-db".to_string()),
		..Default::default()
	};
	let file = file_with_service("db", target);
	let svc = Service {
		network_mode: Some("service:db".to_string()),
		..Default::default()
	};
	let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
	assert_eq!(ns.unwrap().value.as_deref(), Some("custom-db"));
}

#[test]
fn network_mode_service_unknown_target_uses_raw_name() {
	let file = empty_file();
	let svc = Service {
		network_mode: Some("service:missing".to_string()),
		..Default::default()
	};
	let (ns, _) = resolve_network_mode("web", &svc, &file, "proj");
	assert_eq!(ns.unwrap().value.as_deref(), Some("missing"));
}

#[test]
fn resolve_network_mode_no_networks() {
	let svc = Service::default();
	let file = empty_file();
	let (ns, nets) = resolve_network_mode("web", &svc, &file, "proj");
	assert!(ns.is_none());
	assert!(nets.is_empty());
}

#[test]
fn split_network_mode_options_separates_pasta_and_slirp4netns_options() {
	assert_eq!(
		split_network_mode_options("pasta:-T,15432"),
		(
			"pasta",
			Some(("pasta", vec!["-T".to_string(), "15432".to_string()]))
		)
	);
	assert_eq!(
		split_network_mode_options("slirp4netns:allow_host_loopback=true"),
		(
			"slirp4netns",
			Some(("slirp4netns", vec!["allow_host_loopback=true".to_string()]))
		)
	);
	assert_eq!(split_network_mode_options("pasta"), ("pasta", None));
	assert_eq!(split_network_mode_options("pasta:"), ("pasta", None));
	assert_eq!(split_network_mode_options("host"), ("host", None));
	// Only the exact mode name takes options; a longer name is left alone.
	assert_eq!(split_network_mode_options("pastaX:-4"), ("pastaX:-4", None));
}

#[test]
fn resolve_network_mode_sends_the_bare_pasta_mode() {
	let svc = Service {
		network_mode: Some("pasta:-4".to_string()),
		..Default::default()
	};
	let (ns, nets) = resolve_network_mode("web", &svc, &empty_file(), "proj");
	assert_eq!(ns.unwrap().nsmode, "pasta");
	assert!(nets.is_empty());
	assert_eq!(
		network_mode_options(&svc),
		HashMap::from([("pasta".to_string(), vec!["-4".to_string()])])
	);
	assert!(network_mode_options(&Service::default()).is_empty());
}
