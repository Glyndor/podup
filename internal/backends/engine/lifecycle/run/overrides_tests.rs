use super::*;
use crate::compose::types::{Command, Service};

fn words(text: &str) -> Vec<String> {
	split_words(text).unwrap()
}

#[test]
fn words_split_on_whitespace_and_quotes_like_shellwords() {
	assert_eq!(words("sh -c"), ["sh", "-c"]);
	assert_eq!(words("  a   b  "), ["a", "b"]);
	assert_eq!(
		words("sh -c 'echo hi there'"),
		["sh", "-c", "echo hi there"]
	);
	assert_eq!(words(r#"a "b c" d"#), ["a", "b c", "d"]);
	assert_eq!(words(r#""a\"b""#), [r#"a"b"#]);
	assert_eq!(words(r"a\ b"), ["a b"]);
	assert_eq!(words(r#"a''b"#), ["ab"]);
	assert_eq!(words(r#""""#), [""], "an empty quoted word is a word");
	assert!(words("").is_empty());
	assert!(words("   ").is_empty());
}

#[test]
fn an_unterminated_quote_or_trailing_backslash_is_an_error() {
	for bad in ["'abc", "\"abc", "abc\\", "\"abc\\"] {
		assert!(split_words(bad).is_err(), "{bad:?}");
	}
}

fn exec(c: &Option<Command>) -> Option<Vec<String>> {
	c.as_ref().map(Command::to_exec)
}

fn service_with_command() -> Service {
	Service {
		command: Some(Command::Exec(vec!["echo".into(), "old".into()])),
		entrypoint: Some(Command::Exec(vec!["/entry".into()])),
		..Default::default()
	}
}

#[test]
fn an_entrypoint_with_arguments_is_split_and_drops_the_service_command() {
	let mut s = service_with_command();
	apply_entrypoint_and_command(&mut s, Some("sh -c".into()), vec![]).unwrap();
	assert_eq!(exec(&s.entrypoint).unwrap(), ["sh", "-c"]);
	assert_eq!(exec(&s.command).unwrap(), Vec::<String>::new());
}

#[test]
fn an_entrypoint_with_a_command_keeps_the_new_command() {
	let mut s = service_with_command();
	apply_entrypoint_and_command(&mut s, Some("sh -c".into()), vec!["echo hi".into()]).unwrap();
	assert_eq!(exec(&s.entrypoint).unwrap(), ["sh", "-c"]);
	assert_eq!(exec(&s.command).unwrap(), ["echo hi"]);
}

#[test]
fn an_empty_entrypoint_clears_the_image_entrypoint() {
	let mut s = service_with_command();
	apply_entrypoint_and_command(&mut s, Some(String::new()), vec![]).unwrap();
	assert_eq!(exec(&s.entrypoint).unwrap(), [""]);
}

#[test]
fn no_entrypoint_leaves_both_alone_unless_a_command_is_given() {
	let mut s = service_with_command();
	apply_entrypoint_and_command(&mut s, None, vec![]).unwrap();
	assert_eq!(exec(&s.entrypoint).unwrap(), ["/entry"]);
	assert_eq!(exec(&s.command).unwrap(), ["echo", "old"]);
	apply_entrypoint_and_command(&mut s, None, vec!["ls".into()]).unwrap();
	assert_eq!(exec(&s.command).unwrap(), ["ls"]);
	assert_eq!(exec(&s.entrypoint).unwrap(), ["/entry"]);
}

#[test]
fn a_one_off_drops_both_restart_policies() {
	let mut s = Service {
		restart: Some(crate::compose::types::RestartPolicy::Always),
		deploy: serde_yaml::from_str("restart_policy:\n  condition: any\n  max_attempts: 3\n").ok(),
		..Default::default()
	};
	assert!(
		s.deploy.as_ref().unwrap().restart_policy.is_some(),
		"fixture"
	);
	run_once(&mut s);
	assert!(s.restart.is_none());
	assert!(s.deploy.unwrap().restart_policy.is_none());
}
