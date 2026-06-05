use lynx_compose::parse_file;
use std::io::Write;

#[test]
fn include_string_form_merges_services() {
    let dir = tempfile::tempdir().unwrap();

    let included = dir.path().join("services.yml");
    writeln!(
        std::fs::File::create(&included).unwrap(),
        r#"
services:
  helper:
    image: alpine
"#
    )
    .unwrap();

    let main = dir.path().join("docker-compose.yml");
    writeln!(
        std::fs::File::create(&main).unwrap(),
        r#"
include:
  - ./services.yml

services:
  app:
    image: nginx
"#
    )
    .unwrap();

    let file = parse_file(&main).unwrap();
    assert!(file.services.contains_key("app"));
    assert!(file.services.contains_key("helper"));
}

#[test]
fn include_long_form_parses() {
    let dir = tempfile::tempdir().unwrap();

    let inc = dir.path().join("inc.yml");
    writeln!(
        std::fs::File::create(&inc).unwrap(),
        r#"
services:
  inc_svc:
    image: alpine
"#
    )
    .unwrap();

    let main = dir.path().join("docker-compose.yml");
    writeln!(
        std::fs::File::create(&main).unwrap(),
        r#"
include:
  - path: ./inc.yml

services:
  main_svc:
    image: alpine
"#
    )
    .unwrap();

    let file = parse_file(&main).unwrap();
    assert!(file.services.contains_key("inc_svc"));
    assert!(file.services.contains_key("main_svc"));
}

#[test]
fn parent_overrides_included_service() {
    let dir = tempfile::tempdir().unwrap();

    let inc = dir.path().join("inc.yml");
    writeln!(
        std::fs::File::create(&inc).unwrap(),
        r#"
services:
  shared:
    image: alpine:included
"#
    )
    .unwrap();

    let main = dir.path().join("docker-compose.yml");
    writeln!(
        std::fs::File::create(&main).unwrap(),
        r#"
include:
  - ./inc.yml

services:
  shared:
    image: alpine:override
"#
    )
    .unwrap();

    let file = parse_file(&main).unwrap();
    // Parent file definition wins.
    assert_eq!(
        file.services["shared"].image.as_deref(),
        Some("alpine:override")
    );
}
