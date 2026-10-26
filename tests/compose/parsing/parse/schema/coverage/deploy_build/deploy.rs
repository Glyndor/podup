//! The `deploy:` block: restart_policy, update_config, rollback_config,
//! placement and the mode pair.
//!
//! Split out of `tests/compose/parsing/parse/coverage/deploy_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

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
