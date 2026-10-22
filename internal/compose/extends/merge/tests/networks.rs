use super::*;

/// #1078, the one the issue calls worse than a wrong value: a service on
/// `backend` in the base and `monitoring` in the override silently lost
/// `backend`. It dropped off the network and service discovery failed at run
/// time, far from the config that caused it.
#[test]
fn networks_are_unioned_not_replaced() {
	let yaml = r#"
services:
  base:
    image: alpine
    networks:
      - backend
  app:
    extends: base
    networks:
      - monitoring
networks:
  backend:
  monitoring:
"#;
	let file = parse_str(yaml).unwrap();
	let names = file.services["app"].networks.names();
	assert!(
		names.contains(&"backend".to_string()),
		"the base's network must survive: {names:?}"
	);
	assert!(
		names.contains(&"monitoring".to_string()),
		"the override's network must be added: {names:?}"
	);
}

/// A bare name in the override must not erase per-network config the base
/// set: the union keeps the config unless the override supplies its own.
#[test]
fn network_union_keeps_base_config_for_a_bare_override_entry() {
	let yaml = r#"
services:
  base:
    image: alpine
    networks:
      backend:
        aliases:
          - db
  app:
    extends: base
    networks:
      - backend
networks:
  backend:
"#;
	let file = parse_str(yaml).unwrap();
	let cfg = file.services["app"].networks.config_for("backend");
	assert!(
		cfg.is_some_and(|c| c
			.aliases
			.as_ref()
			.is_some_and(|a| a.contains(&"db".to_string()))),
		"a bare override entry must not wipe the base's aliases"
	);
}

/// The union must survive serialization: `config` renders the merged service
/// back to YAML, and a map whose values are all `None` must still emit its
/// network names rather than an empty block.
#[test]
fn unioned_networks_serialize_back_to_their_names() {
	let yaml = r#"
services:
  base:
    image: alpine
    networks:
      - backend
  app:
    extends: base
    networks:
      - monitoring
networks:
  backend:
  monitoring:
"#;
	let file = parse_str(yaml).unwrap();
	let rendered = serde_yaml::to_string(&file.services["app"].networks).unwrap();
	assert!(
		rendered.contains("backend") && rendered.contains("monitoring"),
		"serialized form lost the network names: {rendered}"
	);
}
