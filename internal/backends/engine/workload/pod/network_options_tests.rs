use super::*;

fn options(yaml: &str) -> PodNetworkOptions {
	let file = crate::parse_str(yaml).expect("the fixture parses");
	pod_network_options(&file, &|key| format!("proj_{key}"))
}

const HEAD: &str = "x-podman-pod: true\nservices:\n";

#[test]
fn no_options_means_nothing_applied_and_nothing_said() {
	let o = options(&format!(
		"{HEAD}  a:\n    image: x\n    networks: [n]\n  b:\n    image: x\n    networks: [n]\nnetworks:\n  n: {{}}\n"
	));
	assert!(o.applied.is_empty() && o.warnings.is_empty(), "{o:?}");
}

#[test]
fn an_interface_name_and_driver_options_go_to_the_attachment() {
	let o = options(&format!(
		"{HEAD}  a:\n    image: x\n    networks:\n      n:\n        interface_name: eth7\n        driver_opts:\n          mtu: '1400'\n  b:\n    image: x\n    networks: [n]\nnetworks:\n  n: {{}}\n"
	));
	let applied = o.get("proj_n").unwrap();
	assert_eq!(applied.interface_name.as_deref(), Some("eth7"));
	assert_eq!(applied.driver_opts["mtu"], "1400");
	assert!(o.warnings.is_empty(), "{:?}", o.warnings);
}

/// Two services that ask for the same thing agree.
#[test]
fn services_that_agree_apply_it_once() {
	let o = options(&format!(
		"{HEAD}  a:\n    image: x\n    networks:\n      n:\n        interface_name: eth7\n  b:\n    image: x\n    networks:\n      n:\n        interface_name: eth7\nnetworks:\n  n: {{}}\n"
	));
	assert_eq!(
		o.get("proj_n").unwrap().interface_name.as_deref(),
		Some("eth7")
	);
	assert!(o.warnings.is_empty(), "{:?}", o.warnings);
}

#[test]
fn services_that_disagree_apply_nothing_and_say_who() {
	let o = options(&format!(
		"{HEAD}  a:\n    image: x\n    networks:\n      n:\n        interface_name: eth7\n        driver_opts:\n          mtu: '1400'\n  b:\n    image: x\n    networks:\n      n:\n        interface_name: eth8\n        driver_opts:\n          mtu: '9000'\nnetworks:\n  n: {{}}\n"
	));
	assert!(o.applied.is_empty(), "{o:?}");
	assert_eq!(o.warnings.len(), 2, "{:?}", o.warnings);
	assert!(
		o.warnings.iter().all(|w| w.contains("a, b")),
		"{:?}",
		o.warnings
	);
}

#[test]
fn a_priority_is_reported_not_applied() {
	let o = options(&format!(
		"{HEAD}  a:\n    image: x\n    networks:\n      n:\n        priority: 100\nnetworks:\n  n: {{}}\n"
	));
	assert!(o.applied.is_empty(), "{o:?}");
	assert_eq!(o.warnings.len(), 1, "{:?}", o.warnings);
	assert!(o.warnings[0].contains("service \"a\"") && o.warnings[0].contains("priority"));
}

#[test]
fn apply_puts_them_on_the_pods_attachment_and_leaves_the_others() {
	let o = options(&format!(
		"{HEAD}  a:\n    image: x\n    networks:\n      n:\n        interface_name: eth7\nnetworks:\n  n: {{}}\n  m: {{}}\n"
	));
	let mut nets: HashMap<String, PerNetworkOptions> = HashMap::new();
	nets.entry("proj_n".into()).or_default();
	nets.entry("proj_m".into()).or_default();
	apply(&mut nets, &o);
	assert_eq!(nets["proj_n"].interface_name.as_deref(), Some("eth7"));
	assert!(nets["proj_m"].interface_name.is_none());
}

// --- the pod request and its hash -----------------------------------------

fn spec_and_hash(yaml: &str) -> (serde_json::Value, String) {
	let file = crate::parse_str(yaml).unwrap();
	let ports: Vec<_> = file
		.services
		.values()
		.map(|s| crate::ports::parse_ports(&s.ports).unwrap())
		.collect();
	let hash = super::super::pod_config_hash(&ports, &file);
	let spec = super::super::build_pod_spec_with_hash("demo", &file, &ports, &hash);
	(serde_json::to_value(&spec).unwrap(), hash)
}

const WITH: &str = "x-podman-pod: true\nservices:\n  a:\n    image: x\n    networks:\n      n:\n        interface_name: eth7\n        driver_opts:\n          mtu: '1400'\nnetworks:\n  n: {}\n";
const WITHOUT: &str =
	"x-podman-pod: true\nservices:\n  a:\n    image: x\n    networks: [n]\nnetworks:\n  n: {}\n";

#[test]
fn the_pod_request_carries_the_interface_name_and_the_driver_options() {
	let (body, _) = spec_and_hash(WITH);
	let attachment = &body["networks"]["demo_n"];
	assert_eq!(attachment["interface_name"], "eth7", "{body}");
	assert_eq!(attachment["options"]["mtu"], "1400", "{body}");
}

#[test]
fn a_pod_without_options_sends_an_empty_attachment() {
	let (body, _) = spec_and_hash(WITHOUT);
	assert_eq!(body["networks"]["demo_n"], serde_json::json!({}), "{body}");
}

#[test]
fn the_options_change_the_hash_and_their_absence_leaves_it() {
	let (_, with) = spec_and_hash(WITH);
	let (_, without) = spec_and_hash(WITHOUT);
	assert_ne!(
		with, without,
		"a changed interface name must recreate the pod"
	);
	// An unrelated difference between two projects with no options changes
	// nothing about how the options enter the hash.
	let (_, again) = spec_and_hash(&WITHOUT.replace("image: x", "image: y"));
	assert_eq!(without, again);
}
