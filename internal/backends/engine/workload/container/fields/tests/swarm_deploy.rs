use super::*;

// --- warn_swarm_only_deploy ---

#[test]
fn warn_swarm_only_deploy_no_deploy_is_noop() {
	let svc = default_service();
	warn_swarm_only_deploy("web", &svc);
}

#[test]
fn warn_swarm_only_deploy_no_swarm_fields_is_noop() {
	use crate::compose::types::DeployConfig;
	let mut svc = default_service();
	svc.deploy = Some(DeployConfig {
		replicas: Some(2),
		..Default::default()
	});
	warn_swarm_only_deploy("web", &svc);
}

#[test]
fn warn_swarm_only_deploy_all_swarm_fields_no_panic() {
	use crate::compose::types::{DeployConfig, DeployPlacement, DeployUpdateConfig};
	let mut svc = default_service();
	svc.deploy = Some(DeployConfig {
		mode: Some("global".to_string()),
		placement: Some(DeployPlacement {
			constraints: vec!["node.role == manager".to_string()],
			..Default::default()
		}),
		update_config: Some(DeployUpdateConfig {
			parallelism: Some(1),
			..Default::default()
		}),
		rollback_config: Some(DeployUpdateConfig::default()),
		endpoint_mode: Some("dnsrr".to_string()),
		..Default::default()
	});
	warn_swarm_only_deploy("web", &svc);
}
