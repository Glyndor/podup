use super::*;
use crate::parse_str_raw;

// resolve_order

#[test]
fn resolve_order_no_deps_arbitrary_order() {
	let yaml = "services:\n  a:\n    image: x\n  b:\n    image: y\n";
	let file = parse_str_raw(yaml).unwrap();
	let order = resolve_order(&file).unwrap();
	assert_eq!(order.len(), 2);
	assert!(order.contains(&"a".to_string()));
	assert!(order.contains(&"b".to_string()));
}

#[test]
fn resolve_order_is_deterministic_for_independent_services() {
	// Independent (in-degree-0) services must resolve in a stable,
	// lexicographic order regardless of the HashMap iteration order, so
	// `wait`'s exit code and printed order are reproducible across runs.
	let yaml = "services:\n  c:\n    image: x\n  a:\n    image: y\n  b:\n    image: z\n";
	let file = parse_str_raw(yaml).unwrap();
	let order = resolve_order(&file).unwrap();
	assert_eq!(
		order,
		vec!["a".to_string(), "b".to_string(), "c".to_string()]
	);
	// Re-resolving yields the identical order.
	for _ in 0..16 {
		assert_eq!(resolve_order(&file).unwrap(), order);
	}
}

#[test]
fn resolve_order_dependents_are_deterministic() {
	// Two dependents of the same dependency come out in stable lexicographic
	// order, not whatever the graph's adjacency iteration happens to be.
	let yaml = "services:\n  db:\n    image: x\n  zeb:\n    image: y\n    depends_on: [db]\n  api:\n    image: z\n    depends_on: [db]\n";
	let file = parse_str_raw(yaml).unwrap();
	let order = resolve_order(&file).unwrap();
	assert_eq!(
		order,
		vec!["db".to_string(), "api".to_string(), "zeb".to_string()]
	);
}

#[test]
fn resolve_order_dep_before_dependent() {
	let yaml =
		"services:\n  web:\n    image: nginx\n    depends_on: [db]\n  db:\n    image: postgres\n";
	let file = parse_str_raw(yaml).unwrap();
	let order = resolve_order(&file).unwrap();
	let db_pos = order.iter().position(|s| s == "db").unwrap();
	let web_pos = order.iter().position(|s| s == "web").unwrap();
	assert!(db_pos < web_pos, "db must start before web");
}

#[test]
fn resolve_order_cycle_is_error() {
	let yaml = "services:\n  a:\n    image: x\n    depends_on: [b]\n  b:\n    image: y\n    depends_on: [a]\n";
	let file = parse_str_raw(yaml).unwrap();
	let err = resolve_order(&file).unwrap_err();
	// The message names the services in the cycle and is not the old redundant
	// "circular dependency detected: cycle detected in depends_on".
	let msg = err.to_string();
	assert!(
		msg.contains("circular dependency among services"),
		"got {msg:?}"
	);
	assert!(
		msg.contains('a') && msg.contains('b'),
		"names the cycle: {msg:?}"
	);
	assert!(
		!msg.contains("cycle detected in depends_on"),
		"no redundancy: {msg:?}"
	);
}

#[test]
fn resolve_order_missing_required_dep_is_error() {
	let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n";
	let file = parse_str_raw(yaml).unwrap();
	assert!(resolve_order(&file).is_err());
}

// resolve_levels

#[test]
fn resolve_levels_groups_independent_services_together() {
	let yaml = "services:\n  a:\n    image: x\n  b:\n    image: y\n";
	let file = parse_str_raw(yaml).unwrap();
	let levels = resolve_levels(&file).unwrap();
	// No deps → one level holding both, sorted for determinism.
	assert_eq!(levels, vec![vec!["a".to_string(), "b".to_string()]]);
}

#[test]
fn resolve_levels_orders_dependencies_into_earlier_levels() {
	let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n  db:\n    image: postgres\n  cache:\n    image: redis\n";
	let file = parse_str_raw(yaml).unwrap();
	let levels = resolve_levels(&file).unwrap();
	// Level 0: db + cache (no deps); level 1: web (depends on db).
	assert_eq!(levels[0], vec!["cache".to_string(), "db".to_string()]);
	assert_eq!(levels[1], vec!["web".to_string()]);
}

#[test]
fn resolve_levels_cycle_is_error() {
	let yaml = "services:\n  a:\n    image: x\n    depends_on: [b]\n  b:\n    image: y\n    depends_on: [a]\n";
	let file = parse_str_raw(yaml).unwrap();
	let err = resolve_levels(&file).unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains("circular dependency among services"),
		"got {msg:?}"
	);
	assert!(
		msg.contains('a') && msg.contains('b'),
		"names the cycle: {msg:?}"
	);
}

#[test]
fn resolve_levels_missing_required_dep_is_error() {
	let yaml = "services:\n  web:\n    image: nginx\n    depends_on: [db]\n";
	let file = parse_str_raw(yaml).unwrap();
	assert!(resolve_levels(&file).is_err());
}

#[test]
fn resolve_order_optional_missing_dep_is_ignored() {
	// A `required: false` dependency that is not defined is skipped, not an
	// error; the dependent still resolves.
	let yaml = "services:\n  web:\n    image: nginx\n    depends_on:\n      ghost:\n        condition: service_started\n        required: false\n";
	let file = parse_str_raw(yaml).unwrap();
	let order = resolve_order(&file).unwrap();
	assert_eq!(order, vec!["web".to_string()]);
}

#[test]
fn resolve_levels_optional_missing_dep_is_ignored() {
	let yaml = "services:\n  web:\n    image: nginx\n    depends_on:\n      ghost:\n        condition: service_started\n        required: false\n";
	let file = parse_str_raw(yaml).unwrap();
	let levels = resolve_levels(&file).unwrap();
	assert_eq!(levels, vec![vec!["web".to_string()]]);
}

// Implicit dependencies: docker compose derives these from references that
// need the referenced container to exist first. The names chosen here are
// adversarially sorted: the dependent (`web`/`app`) precedes the
// referent (`zdb`/`zdata`) alphabetically, so a resolver that only reads
// `depends_on` would either start them in the wrong order or fail to
// detect a cycle.

#[test]
fn resolve_levels_orders_network_mode_service_before_dependent() {
	// `web` (network_mode: service:zdb) must wait for `zdb`; alphabetical
	// order would have `web` first, so the only way this passes is if
	// `network_mode: service:X` was treated as an implicit dependency.
	let yaml = "services:\n  web:\n    image: nginx\n    network_mode: \"service:zdb\"\n  zdb:\n    image: postgres\n";
	let file = parse_str_raw(yaml).unwrap();
	let levels = resolve_levels(&file).unwrap();
	let zdb_pos = levels
		.iter()
		.position(|lvl| lvl.iter().any(|s| s == "zdb"))
		.expect("zdb present");
	let web_pos = levels
		.iter()
		.position(|lvl| lvl.iter().any(|s| s == "web"))
		.expect("web present");
	assert!(
		zdb_pos < web_pos,
		"zdb must start before web; got zdb at {zdb_pos}, web at {web_pos}"
	);
}

#[test]
fn resolve_order_puts_referenced_service_before_dependent() {
	// Same adversarial shape, asserted on the linear order.
	let yaml = "services:\n  web:\n    image: nginx\n    network_mode: \"service:zdb\"\n  zdb:\n    image: postgres\n";
	let file = parse_str_raw(yaml).unwrap();
	let order = resolve_order(&file).unwrap();
	let zdb_pos = order.iter().position(|s| s == "zdb").unwrap();
	let web_pos = order.iter().position(|s| s == "web").unwrap();
	assert!(zdb_pos < web_pos, "zdb must start before web");
}

#[test]
fn resolve_levels_orders_volumes_from_before_dependent() {
	let yaml = "services:\n  app:\n    image: nginx\n    volumes_from: [\"zdata\"]\n  zdata:\n    image: postgres\n";
	let file = parse_str_raw(yaml).unwrap();
	let levels = resolve_levels(&file).unwrap();
	let zdata_pos = levels
		.iter()
		.position(|lvl| lvl.iter().any(|s| s == "zdata"))
		.expect("zdata present");
	let app_pos = levels
		.iter()
		.position(|lvl| lvl.iter().any(|s| s == "app"))
		.expect("app present");
	assert!(
		zdata_pos < app_pos,
		"zdata must start before app; got zdata at {zdata_pos}, app at {app_pos}"
	);
}

#[test]
fn resolve_order_volumes_from_cycle_is_error() {
	// docker compose errors with "dependency cycle detected" when two
	// services reference each other through `volumes_from`. The resolver
	// must catch the cycle and name both services in the message.
	let yaml = "services:\n  a:\n    image: x\n    volumes_from: [b]\n  b:\n    image: y\n    volumes_from: [a]\n";
	let file = parse_str_raw(yaml).unwrap();
	let err = resolve_order(&file).unwrap_err();
	let msg = err.to_string();
	assert!(
		msg.contains("circular dependency among services"),
		"got {msg:?}"
	);
	assert!(
		msg.contains('a') && msg.contains('b'),
		"names the cycle: {msg:?}"
	);
}
