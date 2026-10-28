use super::*;

#[test]
fn extends_unions_sequence_fields() {
	let yaml = r#"
services:
  base:
    image: alpine
    ports:
      - "80:80"
      - "81:81"
  app:
    extends: base
    ports:
      - "90:90"
"#;
	let file = parse_str(yaml).unwrap();
	// Compose `extends` combines sequences (base first, then the extending
	// service's items) rather than replacing the base wholesale.
	assert_eq!(file.services["app"].ports.len(), 3);
}

#[test]
fn extends_dedups_identical_sequence_entries() {
	let yaml = r#"
services:
  base:
    image: alpine
    ports:
      - "80:80"
  app:
    extends: base
    ports:
      - "80:80"
      - "90:90"
"#;
	let file = parse_str(yaml).unwrap();
	// An exact duplicate from the extending service is dropped.
	assert_eq!(file.services["app"].ports.len(), 2);
}

#[test]
fn absent_list_field_falls_back_to_base() {
	let yaml = r#"
services:
  base:
    image: alpine
    ports:
      - "80:80"
  app:
    extends: base
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(file.services["app"].ports.len(), 1);
}

#[test]
fn labels_are_merged_with_override_winning() {
	let yaml = r#"
services:
  base:
    image: alpine
    labels:
      a: base
      keep: base
  app:
    extends: base
    labels:
      a: over
      b: over
"#;
	let file = parse_str(yaml).unwrap();
	let labels = file.services["app"].labels.to_map();
	assert_eq!(labels.get("a").map(|s| s.as_str()), Some("over"));
	assert_eq!(labels.get("keep").map(|s| s.as_str()), Some("base"));
	assert_eq!(labels.get("b").map(|s| s.as_str()), Some("over"));
}

#[test]
fn absent_override_keeps_base_environment() {
	let yaml = r#"
services:
  base:
    image: alpine
    environment:
      A: "1"
  app:
    extends: base
"#;
	let file = parse_str(yaml).unwrap();
	let env = file.services["app"].environment.to_map();
	assert_eq!(env.get("A").and_then(|v| v.clone()).as_deref(), Some("1"));
}

/// #1078: `dns` and its siblings are appended, not replaced. docker compose
/// concatenates these sequences; replacing meant an override adding one
/// nameserver silently removed every other one.
#[test]
fn scalar_or_list_field_override_appends_to_base() {
	let yaml = r#"
services:
  base:
    image: alpine
    dns:
      - 1.1.1.1
  app:
    extends: base
    dns:
      - 9.9.9.9
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.services["app"].dns.to_list(),
		vec!["1.1.1.1", "9.9.9.9"],
		"the base's nameserver must survive an override that adds one"
	);
}

/// #1078: `env_file` is appended too: the base's files are still read, in
/// order, followed by the override's.
#[test]
fn env_file_override_appends_to_base() {
	let yaml = r#"
services:
  base:
    image: alpine
    env_file:
      - base.env
  app:
    extends: base
    env_file:
      - app.env
"#;
	let file = parse_str(yaml).unwrap();
	let entries = file.services["app"].env_file.to_entries();
	assert_eq!(entries.len(), 2, "both env files must be read");
	assert_eq!(entries[0].path(), "base.env");
	assert_eq!(entries[1].path(), "app.env");
}

/// #1078: `sysctls` merges per key like `environment` and `labels`, rather
/// than the override replacing the whole map.
#[test]
fn sysctls_merge_per_key() {
	let yaml = r#"
services:
  base:
    image: alpine
    sysctls:
      net.core.somaxconn: "1024"
      net.ipv4.tcp_syncookies: "1"
  app:
    extends: base
    sysctls:
      net.core.somaxconn: "4096"
"#;
	let file = parse_str(yaml).unwrap();
	let m = file.services["app"].sysctls.to_map();
	assert_eq!(
		m.get("net.core.somaxconn").map(String::as_str),
		Some("4096"),
		"the override wins for a key both set"
	);
	assert_eq!(
		m.get("net.ipv4.tcp_syncookies").map(String::as_str),
		Some("1"),
		"a key only the base sets must survive"
	);
}

/// #1078: a base and an override mounting different sources at the *same*
/// target used to emit both, and `podman create` refused the container with
/// `duplicate mount destination`. Remapping a path in an override is exactly
/// what the override is for, so the later one wins.
#[test]
fn volumes_at_the_same_target_are_replaced_not_duplicated() {
	let yaml = r#"
services:
  base:
    image: alpine
    volumes:
      - ./a:/data
      - ./logs:/var/log
  app:
    extends: base
    volumes:
      - ./b:/data
"#;
	let file = parse_str(yaml).unwrap();
	let mounts = &file.services["app"].volumes;
	let at_data: Vec<_> = mounts.iter().filter(|m| m.target() == "/data").collect();
	assert_eq!(at_data.len(), 1, "one mount per target: {mounts:?}");
	assert!(
		format!("{:?}", at_data[0]).contains("./b"),
		"the override's source must win: {:?}",
		at_data[0]
	);
	// A target only the base declares is untouched.
	assert!(
		mounts.iter().any(|m| m.target() == "/var/log"),
		"an unrelated base mount must survive: {mounts:?}"
	);
}

/// `pull_refresh_after:` is inherited from the base service and overridden by
/// the service that extends it, like every other scalar field. Retaining the
/// declared interval lets validation catch malformed inherited values even
/// though the runner otherwise ignores valid intervals.
#[test]
fn pull_refresh_after_is_inherited_and_overridden() {
	let yaml = r#"
services:
  base:
    image: alpine
    pull_policy: refresh
    pull_refresh_after: 24h
  app:
    extends: base
  other:
    extends: base
    pull_refresh_after: 1h
"#;
	let file = parse_str(yaml).unwrap();
	assert_eq!(
		file.services["app"].pull_refresh_after.as_deref(),
		Some("24h"),
		"the base's window must be inherited"
	);
	assert_eq!(
		file.services["other"].pull_refresh_after.as_deref(),
		Some("1h"),
		"the extending service's window must win"
	);
}

fn merged(yaml: &str) -> crate::compose::types::ComposeFile {
	parse_str(yaml).unwrap()
}

#[test]
fn an_override_that_sets_one_resource_limit_keeps_the_other() {
	let file = merged(
		r#"
services:
  base:
    image: alpine
    deploy:
      resources:
        limits:
          cpus: "0.5"
  app:
    extends: base
    deploy:
      resources:
        limits:
          memory: 256M
"#,
	);
	let limits = file.services["app"]
		.deploy
		.as_ref()
		.and_then(|d| d.resources.as_ref())
		.and_then(|r| r.limits.as_ref())
		.expect("limits");
	assert!(limits.cpus.is_some(), "the base cpus limit was lost");
	assert!(
		limits.memory.is_some(),
		"the override memory limit was lost"
	);
}

#[test]
fn an_override_that_sets_one_healthcheck_field_keeps_the_test() {
	let file = merged(
		r#"
services:
  base:
    image: alpine
    healthcheck:
      test: ["CMD", "true"]
  app:
    extends: base
    healthcheck:
      interval: 5s
"#,
	);
	let health = file.services["app"]
		.healthcheck
		.as_ref()
		.expect("healthcheck");
	assert!(health.test.is_some(), "the base test was lost");
	assert!(health.interval.is_some(), "the override interval was lost");
}

#[test]
fn an_override_that_sets_build_args_keeps_the_context() {
	let file = merged(
		r#"
services:
  base:
    build:
      context: ./app
  app:
    extends: base
    build:
      args:
        MODE: release
"#,
	);
	let build = file.services["app"].build.as_ref().expect("build");
	assert_eq!(build.context(), "./app", "the base context was lost");
	assert!(!build.args().is_empty(), "the override args were lost");
}
