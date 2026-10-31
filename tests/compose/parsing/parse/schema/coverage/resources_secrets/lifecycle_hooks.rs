//! The `post_start` and `pre_stop` lifecycle hooks.
//!
//! Split out of `tests/compose/parsing/parse/schema/coverage/resources_secrets/mod.rs`; the imports
//! are inherited via `use super::*`.

use super::*;

// ---------------------------------------------------------------------------
// post_start / pre_stop lifecycle hooks
// ---------------------------------------------------------------------------

#[test]
fn post_start_exec_list() {
	let yaml = r#"
services:
  app:
    image: alpine
    post_start:
      - command: ["/scripts/init.sh", "--quiet"]
"#;
	let svc = &parse_str(yaml).unwrap().services["app"];
	assert_eq!(svc.post_start.len(), 1);
	let cmd = svc.post_start[0].command.to_exec();
	assert_eq!(cmd[0], "/scripts/init.sh");
	assert_eq!(cmd[1], "--quiet");
}

#[test]
fn pre_stop_with_env_and_user() {
	let yaml = r#"
services:
  app:
    image: alpine
    pre_stop:
      - command: ["/scripts/cleanup.sh"]
        user: "1000"
        privileged: false
        working_dir: /app
        environment:
          CLEANUP: "true"
"#;
	let hook = &parse_str(yaml).unwrap().services["app"].pre_stop[0];
	assert_eq!(hook.user.as_deref(), Some("1000"));
	assert_eq!(hook.privileged, Some(false));
	assert_eq!(hook.working_dir.as_deref(), Some("/app"));
	let env = hook.environment.to_map();
	assert_eq!(
		env.get("CLEANUP").and_then(|v| v.clone()).as_deref(),
		Some("true")
	);
}
