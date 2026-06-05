use podup::compose::types::PortMapping;
use podup::ports::{parse_ports, to_bollard};

fn short(s: &str) -> PortMapping {
    PortMapping::Short(s.to_string())
}

#[test]
fn bollard_keys_and_bindings() {
    let ports = parse_ports(&[short("8080:80")]).unwrap();
    let (bindings, exposed) = to_bollard(&ports);
    assert!(bindings.contains_key("80/tcp"));
    assert!(exposed.contains_key("80/tcp"));
    let b = bindings["80/tcp"].as_ref().unwrap();
    assert_eq!(b[0].host_port.as_deref(), Some("8080"));
    assert_eq!(b[0].host_ip.as_deref(), Some("0.0.0.0"));
}

#[test]
fn bollard_udp_key() {
    let ports = parse_ports(&[short("514:514/udp")]).unwrap();
    let (bindings, _) = to_bollard(&ports);
    assert!(bindings.contains_key("514/udp"));
}

#[test]
fn bollard_range_produces_multiple_keys() {
    let ports = parse_ports(&[short("8000-8001:8000-8001")]).unwrap();
    let (bindings, _) = to_bollard(&ports);
    assert!(bindings.contains_key("8000/tcp"));
    assert!(bindings.contains_key("8001/tcp"));
}
