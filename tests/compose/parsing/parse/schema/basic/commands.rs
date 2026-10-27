//! `command` and `entrypoint`, in the shell string form and in the exec list
//! form, both reaching the same exec vector.
//!
//! Split out of `tests/compose/parsing/parse/schema/basic/mod.rs`; the imports are inherited via
//! `use super::*`.

use super::*;

#[test]
fn command_shell() {
	let yaml = r#"
services:
  app:
    image: node:20
    command: "node server.js --port 3000"
"#;
	let exec = parse_str(yaml).unwrap().services["app"]
		.command
		.as_ref()
		.unwrap()
		.to_exec();
	assert_eq!(exec[0], "sh");
	assert_eq!(exec[1], "-c");
}

#[test]
fn command_exec() {
	let yaml = r#"
services:
  app:
    image: node:20
    command: ["node", "server.js"]
"#;
	let exec = parse_str(yaml).unwrap().services["app"]
		.command
		.as_ref()
		.unwrap()
		.to_exec();
	assert_eq!(exec, vec!["node", "server.js"]);
}

#[test]
fn entrypoint_shell() {
	let yaml = r#"
services:
  app:
    image: alpine
    entrypoint: "/usr/local/bin/init.sh --foo"
"#;
	let ep = parse_str(yaml).unwrap().services["app"]
		.entrypoint
		.as_ref()
		.unwrap()
		.to_exec();
	assert_eq!(ep[0], "sh");
	assert_eq!(ep[1], "-c");
	assert!(ep[2].contains("init.sh"));
}

#[test]
fn entrypoint_exec() {
	let yaml = r#"
services:
  app:
    image: alpine
    entrypoint: ["/bin/init", "--foo"]
"#;
	let ep = parse_str(yaml).unwrap().services["app"]
		.entrypoint
		.as_ref()
		.unwrap()
		.to_exec();
	assert_eq!(ep, vec!["/bin/init", "--foo"]);
}
