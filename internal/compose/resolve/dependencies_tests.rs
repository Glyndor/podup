use super::*;
use crate::compose::types::{DependsOn, ServiceCondition};
use crate::parse_str;

fn file(yaml: &str) -> crate::compose::types::ComposeFile {
	parse_str(yaml).expect("yaml parses")
}

fn service<'a>(
	file: &'a crate::compose::types::ComposeFile,
	name: &str,
) -> &'a crate::compose::types::Service {
	file.services.get(name).expect("service present")
}

#[test]
fn volumes_from_bare_adds_implicit_entry() {
	let f =
		file("services:\n  data:\n    image: x\n  ro:\n    image: x\n    volumes_from: [data]\n");
	let d = effective_depends_on(service(&f, "ro"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "data"));
	assert_eq!(d.condition_for("data"), ServiceCondition::ServiceStarted);
	assert!(d.required_for("data"));
	assert!(!d.restart_for("data"));
}

#[test]
fn volumes_from_ro_mode_adds_implicit_entry() {
	let f = file(
		"services:\n  data:\n    image: x\n  ro:\n    image: x\n    volumes_from: [data:ro]\n",
	);
	let d = effective_depends_on(service(&f, "ro"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "data"));
	assert_eq!(d.condition_for("data"), ServiceCondition::ServiceStarted);
	assert!(d.required_for("data"));
	assert!(!d.restart_for("data"));
}

#[test]
fn volumes_from_service_prefix_adds_implicit_entry() {
	let f = file(
		"services:\n  data:\n    image: x\n  ro:\n    image: x\n    volumes_from: [\"service:data:ro\"]\n",
	);
	let d = effective_depends_on(service(&f, "ro"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "data"));
	assert!(!d.restart_for("data"));
}

#[test]
fn volumes_from_container_form_produces_no_entry() {
	let f = file(
		"services:\n  data:\n    image: x\n  ro:\n    image: x\n    volumes_from: [\"container:data\"]\n",
	);
	let d = effective_depends_on(service(&f, "ro"), &f.services);
	assert!(d.service_names().is_empty());
}

#[test]
fn volumes_from_unknown_service_produces_no_entry() {
	let f = file("services:\n  ro:\n    image: x\n    volumes_from: [\"ghost\"]\n");
	let d = effective_depends_on(service(&f, "ro"), &f.services);
	assert!(d.service_names().is_empty());
}

#[test]
fn links_with_alias_adds_implicit_entry_with_restart() {
	let f = file(
		"services:\n  data:\n    image: x\n  web:\n    image: x\n    links: [\"data:alias\"]\n",
	);
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "data"));
	assert!(d.restart_for("data"));
}

#[test]
fn links_bare_adds_implicit_entry_with_restart() {
	let f = file("services:\n  data:\n    image: x\n  web:\n    image: x\n    links: [data]\n");
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "data"));
	assert!(d.restart_for("data"));
}

#[test]
fn network_mode_service_adds_implicit_entry_with_restart() {
	let f = file(
		"services:\n  zdb:\n    image: x\n  web:\n    image: x\n    network_mode: \"service:zdb\"\n",
	);
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "zdb"));
	assert!(d.restart_for("zdb"));
}

#[test]
fn ipc_service_adds_implicit_entry_with_restart() {
	let f =
		file("services:\n  zdb:\n    image: x\n  web:\n    image: x\n    ipc: \"service:zdb\"\n");
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "zdb"));
	assert!(d.restart_for("zdb"));
}

#[test]
fn pid_service_adds_implicit_entry_with_restart() {
	let f =
		file("services:\n  zdb:\n    image: x\n  web:\n    image: x\n    pid: \"service:zdb\"\n");
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "zdb"));
	assert!(d.restart_for("zdb"));
}

#[test]
fn uts_service_adds_implicit_entry_with_restart() {
	let f =
		file("services:\n  zdb:\n    image: x\n  web:\n    image: x\n    uts: \"service:zdb\"\n");
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(d.service_names().iter().any(|n| n == "zdb"));
	assert!(d.restart_for("zdb"));
}

#[test]
fn network_mode_container_produces_no_entry() {
	let f = file("services:\n  web:\n    image: x\n    network_mode: \"container:data\"\n");
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(d.service_names().is_empty());
}

#[test]
fn network_mode_host_produces_no_entry() {
	let f = file("services:\n  web:\n    image: x\n    network_mode: \"host\"\n");
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(d.service_names().is_empty());
}

#[test]
fn explicit_depends_on_wins_over_implicit() {
	// User-set condition is preserved; the implicit entry still augments the map.
	let f = file(
		"services:\n  data:\n    image: x\n  ro:\n    image: x\n    volumes_from: [data]\n    depends_on:\n      data:\n        condition: service_healthy\n        required: false\n",
	);
	let d = effective_depends_on(service(&f, "ro"), &f.services);
	assert_eq!(d.condition_for("data"), ServiceCondition::ServiceHealthy);
	assert!(!d.required_for("data"));
}

#[test]
fn explicit_list_plus_implicit_combines_into_map() {
	let f = file(
		"services:\n  a:\n    image: x\n  b:\n    image: x\n  web:\n    image: x\n    depends_on: [a]\n    links: [b]\n",
	);
	let d = effective_depends_on(service(&f, "web"), &f.services);
	let names = d.service_names();
	assert_eq!(names.len(), 2);
	assert!(names.contains(&"a".to_string()));
	assert!(names.contains(&"b".to_string()));
	// The list form maps to `condition: service_started` and `required: true`
	// with `restart: None`; the `links` entry is the one that carries `restart: true`.
	assert_eq!(d.condition_for("a"), ServiceCondition::ServiceStarted);
	assert!(d.required_for("a"));
	assert!(!d.restart_for("a"));
	assert!(d.restart_for("b"));
}

/// The parser must not mutate `Service::depends_on` at load time. If a
/// future change moved the implicit-dependency rule into the loader, this
/// assertion would fail: the loader would have augmented the service's
/// own `depends_on` rather than leaving it `Empty`.
#[test]
fn loader_does_not_mutate_depends_on() {
	let f = parse_str("services:\n  ro:\n    image: x\n    volumes_from: [data]\n")
		.expect("yaml parses");
	assert!(
		matches!(f.services["ro"].depends_on, DependsOn::Empty),
		"Service::depends_on must stay Empty after parsing; got {:?}",
		f.services["ro"].depends_on
	);
}

#[test]
fn empty_implicit_returns_explicit_unchanged() {
	let f = file("services:\n  web:\n    image: x\n    depends_on: [a]\n  a:\n    image: x\n");
	let d = effective_depends_on(service(&f, "web"), &f.services);
	// Explicit list form preserved exactly (List, not converted to a Map).
	assert!(matches!(d, DependsOn::List(_)));
	assert_eq!(d.service_names(), vec!["a".to_string()]);
}

/// An explicit list-form `depends_on: [data]` plus `links: [data]` must
/// keep `restart` false on the explicit entry. docker compose v5.1.3
/// leaves the explicit entry untouched when `links` also points at the
/// same service; the implicit `links` rule does not silently flip the
/// `restart` flag the user did not set.
#[test]
fn explicit_list_plus_links_keeps_restart_false() {
	let f = file(
		"services:\n  data:\n    image: x\n  web:\n    image: x\n    depends_on: [data]\n    links: [data]\n",
	);
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert_eq!(d.condition_for("data"), ServiceCondition::ServiceStarted);
	assert!(d.required_for("data"));
	assert!(!d.restart_for("data"));
}

/// An explicit map-form `depends_on: {data: {condition: service_started}}`
/// plus `network_mode: "service:data"` must keep `restart` false on the
/// explicit entry. Same docker compose rule as the list case: the implicit
/// namespace reference does not promote `restart` on a key the user wrote.
#[test]
fn explicit_map_plus_network_mode_service_keeps_restart_false() {
	let f = file(
		"services:\n  data:\n    image: x\n  web:\n    image: x\n    depends_on:\n      data:\n        condition: service_started\n    network_mode: \"service:data\"\n",
	);
	let d = effective_depends_on(service(&f, "web"), &f.services);
	assert!(!d.restart_for("data"));
}
