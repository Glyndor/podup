use super::unit_named;
use crate::parse_str;
use crate::quadlet::generate_at;

#[test]
fn ephemeral_published_port_omits_host_side() {
	let yaml = r#"
services:
  s:
    image: x
    ports:
      - "80"
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("PublishPort=80"));
	assert!(!c.contains("PublishPort=:80"));
}

#[test]
fn static_ips_render_as_ip_keys() {
	// `networks.<n>.ipv4_address`/`ipv6_address` must reach the unit as IP=/IP6=,
	// not be silently dropped.
	let yaml = r#"
services:
  s:
    image: x
    networks:
      net:
        ipv4_address: 10.5.0.7
        ipv6_address: "2001:db8::7"
networks:
  net:
"#;
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("IP=10.5.0.7"), "missing IP= in:\n{c}");
	assert!(c.contains("IP6=2001:db8::7"), "missing IP6= in:\n{c}");
}

#[test]
fn network_mode_none_maps_to_network_none() {
	// `network_mode: none` is a valid Quadlet value; it must map to Network=none
	// rather than being warned and dropped.
	let yaml = "services:\n  s:\n    image: x\n    network_mode: none\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("Network=none"), "missing Network=none in:\n{c}");
	assert!(
		!out.warnings.iter().any(|w| w.contains("network_mode")),
		"network_mode: none must not warn; got: {:?}",
		out.warnings
	);
}

#[test]
fn network_mode_pasta_with_options_passes_through_to_network() {
	// `Network=` takes the same value as `podman run --network`, so a pasta
	// mode with options is written as is instead of being warned and dropped
	// (#1994).
	let yaml = "services:\n  s:\n    image: x\n    network_mode: \"pasta:-T,15432\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(
		c.contains("Network=pasta:-T,15432"),
		"missing Network= in:\n{c}"
	);
	assert!(
		!out.warnings.iter().any(|w| w.contains("network_mode")),
		"a pasta network_mode must not warn; got: {:?}",
		out.warnings
	);
}

#[test]
fn network_mode_pasta_is_not_written_for_a_pod_member() {
	// A pod member uses the pod's namespace, and Podman refuses a second
	// network for it, so `Network=` must not appear next to `Pod=`.
	let yaml =
		"x-podman-pod: true\nservices:\n  s:\n    image: x\n    network_mode: \"pasta:-4\"\n";
	let file = parse_str(yaml).unwrap();
	let out = generate_at(&file, "p", std::path::Path::new("/srv/app"));
	let c = &unit_named(&out, "p-s.container").contents;
	assert!(c.contains("Pod="), "expected a pod member in:\n{c}");
	assert!(
		!c.contains("Network=pasta"),
		"pod member got Network= in:\n{c}"
	);
}
