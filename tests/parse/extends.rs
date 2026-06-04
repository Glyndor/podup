use lynx_compose::{parse_file, parse_str};
use std::io::Write;

#[test]
fn extends_same_file() {
    let yaml = r#"
services:
  base:
    image: alpine
    environment:
      LOG: info
  app:
    extends: base
"#;
    let file = parse_str(yaml).unwrap();
    assert_eq!(file.services["app"].image.as_deref(), Some("alpine"));
}

#[test]
fn extends_merges_environment() {
    let yaml = r#"
services:
  base:
    image: alpine
    environment:
      LOG: info
      KEEP: yes
  app:
    extends:
      service: base
    environment:
      LOG: debug
      EXTRA: ok
"#;
    let file = parse_str(yaml).unwrap();
    let env = file.services["app"].environment.to_map();
    // Override wins for LOG, base value preserved for KEEP, override-only EXTRA present.
    assert_eq!(
        env.get("LOG").and_then(|v| v.clone()).as_deref(),
        Some("debug")
    );
    assert_eq!(
        env.get("KEEP").and_then(|v| v.clone()).as_deref(),
        Some("yes")
    );
    assert_eq!(
        env.get("EXTRA").and_then(|v| v.clone()).as_deref(),
        Some("ok")
    );
}

#[test]
fn extends_override_image_wins() {
    let yaml = r#"
services:
  base:
    image: alpine
  app:
    extends: base
    image: nginx:alpine
"#;
    let file = parse_str(yaml).unwrap();
    assert_eq!(file.services["app"].image.as_deref(), Some("nginx:alpine"));
}

#[test]
fn extends_chains_through_multiple_levels() {
    let yaml = r#"
services:
  grand:
    image: alpine
    environment:
      A: 1
  parent:
    extends: grand
    environment:
      B: 2
  child:
    extends: parent
    environment:
      C: 3
"#;
    let file = parse_str(yaml).unwrap();
    let env = file.services["child"].environment.to_map();
    assert!(env.contains_key("A"));
    assert!(env.contains_key("B"));
    assert!(env.contains_key("C"));
    assert_eq!(file.services["child"].image.as_deref(), Some("alpine"));
}

#[test]
fn extends_circular_errors() {
    let yaml = r#"
services:
  a:
    image: alpine
    extends: b
  b:
    image: alpine
    extends: a
"#;
    assert!(parse_str(yaml).is_err());
}

#[test]
fn extends_self_errors() {
    let yaml = r#"
services:
  a:
    image: alpine
    extends: a
"#;
    assert!(parse_str(yaml).is_err());
}

#[test]
fn extends_unknown_service_errors() {
    let yaml = r#"
services:
  a:
    image: alpine
    extends: missing
"#;
    assert!(parse_str(yaml).is_err());
}

#[test]
fn extends_with_external_file() {
    let dir = tempfile::tempdir().unwrap();
    let common_path = dir.path().join("common.yml");
    let mut f = std::fs::File::create(&common_path).unwrap();
    writeln!(
        f,
        r#"
services:
  base:
    image: alpine
    environment:
      FROM_BASE: yes
"#
    )
    .unwrap();

    let main_path = dir.path().join("docker-compose.yml");
    let mut m = std::fs::File::create(&main_path).unwrap();
    writeln!(
        m,
        r#"
services:
  app:
    extends:
      service: base
      file: ./common.yml
    environment:
      FROM_APP: yes
"#
    )
    .unwrap();

    let file = parse_file(&main_path).unwrap();
    assert_eq!(file.services["app"].image.as_deref(), Some("alpine"));
    let env = file.services["app"].environment.to_map();
    assert!(env.contains_key("FROM_BASE"));
    assert!(env.contains_key("FROM_APP"));
}
