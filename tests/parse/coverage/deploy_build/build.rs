//! The long-form `build:` block: no_cache, pull, tags, privileged,
//! extra_hosts, additional_contexts, dockerfile_inline, ssh and secrets.
//!
//! Split out of `tests/parse/coverage/deploy_build/mod.rs`; the imports are
//! inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// Build: no_cache, pull, tags, privileged, extra_hosts, additional_contexts,
//        dockerfile_inline, ssh, secrets, ulimits
// ---------------------------------------------------------------------------

#[test]
fn build_no_cache_and_pull() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      no_cache: true
      pull: true
"#;
	let file = parse_str(yaml).unwrap();
	let build = file.services["app"].build.as_ref().unwrap();
	assert!(build.no_cache());
	assert!(build.pull());
}

#[test]
fn build_tags() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      tags:
        - myregistry.io/myapp:v1.2.3
        - myregistry.io/myapp:latest
"#;
	let file = parse_str(yaml).unwrap();
	let build = file.services["app"].build.as_ref().unwrap();
	assert_eq!(build.tags().len(), 2);
	assert!(build.tags()[0].contains("v1.2.3"));
}

#[test]
fn build_privileged() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      privileged: true
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config { privileged, .. } => assert_eq!(*privileged, Some(true)),
		_ => panic!("expected long-form build"),
	}
}

#[test]
fn build_extra_hosts() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      extra_hosts:
        - "somehost:162.242.195.82"
"#;
	let file = parse_str(yaml).unwrap();
	let build = file.services["app"].build.as_ref().unwrap();
	assert_eq!(build.extra_hosts().len(), 1);
}

#[test]
fn build_additional_contexts() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      additional_contexts:
        mylib: /path/to/mylib
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config {
			additional_contexts,
			..
		} => {
			assert_eq!(
				additional_contexts.get("mylib").map(|s| s.as_str()),
				Some("/path/to/mylib")
			);
		}
		_ => panic!("expected long-form build"),
	}
}

#[test]
fn build_dockerfile_inline() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      dockerfile_inline: |
        FROM alpine
        RUN echo hello
"#;
	let file = parse_str(yaml).unwrap();
	let build = file.services["app"].build.as_ref().unwrap();
	let inline = build.dockerfile_inline().unwrap();
	assert!(inline.contains("FROM alpine"));
}

#[test]
fn build_ssh_and_secrets() {
	let yaml = r#"
services:
  app:
    build:
      context: .
      ssh:
        - default
      secrets:
        - server-certificate
"#;
	match parse_str(yaml).unwrap().services["app"]
		.build
		.as_ref()
		.unwrap()
	{
		BuildConfig::Config { ssh, secrets, .. } => {
			assert_eq!(ssh.len(), 1);
			assert_eq!(secrets.len(), 1);
			assert_eq!(secrets[0], "server-certificate");
		}
		_ => panic!("expected long-form build"),
	}
}
