//! `ports`, `networks` and `expose`.
//!
//! Split out of `tests/compose/parsing/parse/schema/basic/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn ports_short() {
	let yaml = r#"
services:
  web:
    image: nginx
    ports: ["80:80", "443:443"]
"#;
	assert_eq!(parse_str(yaml).unwrap().services["web"].ports.len(), 2);
}

#[test]
fn networks_list() {
	let yaml = r#"
services:
  web:
    image: nginx
    networks: [frontend]
networks:
  frontend:
    driver: bridge
"#;
	let file = parse_str(yaml).unwrap();
	assert!(file.networks.contains_key("frontend"));
	assert_eq!(file.services["web"].networks.names(), vec!["frontend"]);
}

#[test]
fn expose_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    expose: ["80", "443"]
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].expose, vec!["80", "443"]);
}
