//! A string `command:` or `entrypoint:` is split into words the way Docker
//! Compose splits it, with no `sh -c` around it. The expected words in `TABLE`
//! are what `docker compose config --format json` (v5.7.0) printed for the
//! same string, `None` where it refused the file with "invalid command line
//! string". A string with a `$` is not here: Compose interpolates it before it
//! splits, which the loader tests cover.

use super::super::service::Service;

const TABLE: &[(&str, Option<&[&str]>)] = &[
	("echo hi", Some(&["echo", "hi"])),
	("  a   b  ", Some(&["a", "b"])),
	("a\tb", Some(&["a", "b"])),
	("a\r\nb", Some(&["a", "b"])),
	("a\nb", Some(&["a", "b"])),
	(
		"sh -c 'echo hi there'",
		Some(&["sh", "-c", "echo hi there"]),
	),
	("a \"b c\" d", Some(&["a", "b c", "d"])),
	("\"a\\\"b\"", Some(&["a\"b"])),
	("a\\ b", Some(&["a b"])),
	("a''b", Some(&["ab"])),
	("\"\"", Some(&[""])),
	("\"\"\"\"", Some(&[""])),
	("a '' b", Some(&["a", "", "b"])),
	("", Some(&[])),
	("   ", Some(&[])),
	("  \t ", Some(&[])),
	("'a\\nb'", Some(&["a\\nb"])),
	("\"a\\nb\"", Some(&["a\nb"])),
	("\"a\\tb\"", Some(&["a\tb"])),
	("a\\nb", Some(&["a\nb"])),
	("a\\tb", Some(&["a\tb"])),
	("\"a\\\\b\"", Some(&["a\\b"])),
	("\"a\\qb\"", Some(&["aqb"])),
	("a\\rb", Some(&["arb"])),
	("'a\\\\b'", Some(&["a\\\\b"])),
	("a 'b\\'", Some(&["a", "b\\"])),
	("a\"b c\"d", Some(&["ab cd"])),
	("x 'y z'w", Some(&["x", "y zw"])),
	("é ü", Some(&["é", "ü"])),
	("a\u{a0}b", Some(&["a\u{a0}b"])),
	("a\u{b}b", Some(&["a\u{b}b"])),
	("a\u{2003}b", Some(&["a\u{2003}b"])),
	("echo a;b && c", Some(&["echo", "a"])),
	("a && b", Some(&["a"])),
	("a | b", Some(&["a"])),
	("a > out", Some(&["a"])),
	("a < in", Some(&["a"])),
	("a ; b", Some(&["a"])),
	("a;", Some(&["a"])),
	("; a", Some(&[])),
	("a&b", Some(&["a"])),
	("a 2>err", Some(&["a"])),
	("\"a\";b", Some(&["a"])),
	("a;b;c", Some(&["a"])),
	("a \";\" b", Some(&["a", ";", "b"])),
	("a ';' b", Some(&["a", ";", "b"])),
	("a\\;b", Some(&["a;b"])),
	("a #c", Some(&["a", "#c"])),
	("a # c", Some(&["a", "#", "c"])),
	("echo `da te`", Some(&["echo", "`da te`"])),
	("{a,b}", Some(&["{a,b}"])),
	("a*", Some(&["a*"])),
	("~ x", Some(&["~", "x"])),
	("a=b c", Some(&["a=b", "c"])),
	("'unterminated", None),
	("\"unterminated", None),
	("trailing\\", None),
	("\"a\\", None),
	("(", None),
	("a)", None),
	("a (b) c", None),
	("\"(\" x", Some(&["(", "x"])),
	("'(' x", Some(&["(", "x"])),
	("a\\(b", Some(&["a(b"])),
	("a ; (b", Some(&["a"])),
	("a 2>", Some(&["a"])),
	("a 22>x", Some(&["a"])),
	("a x2>", Some(&["a", "x2"])),
	("a 2<x", Some(&["a", "2"])),
	("a 2 >x", Some(&["a", "2"])),
	("a 2&x", Some(&["a", "2"])),
	("a 2|x", Some(&["a", "2"])),
	("a 2;", Some(&["a", "2"])),
	("a 2>&1", Some(&["a"])),
	("a 2x>", Some(&["a", "2x"])),
	("a 0>x", Some(&["a"])),
	("a '2'>x", Some(&["a", "2"])),
	("a \"2\">x", Some(&["a", "2"])),
	("a \\2>x", Some(&["a", "2"])),
	("2>x", Some(&[])),
	("a 2>>x", Some(&["a"])),
	("a -2>x", Some(&["a", "-2"])),
	("a 1.5>x", Some(&["a", "1.5"])),
	("a 2", Some(&["a", "2"])),
	("a 12", Some(&["a", "12"])),
	("a 12;", Some(&["a", "12"])),
	("a 2>x", Some(&["a"])),
	("a 5 > 6", Some(&["a", "5"])),
	("echo `a;b`", Some(&["echo", "`a;b`"])),
	("echo `a", None),
	("echo \"`a b`\"", Some(&["echo", "`a b`"])),
	("echo '`a b'", Some(&["echo", "`a b"])),
	("echo `a\\ b`", Some(&["echo", "`a\\ b`"])),
	("echo `a'b`", Some(&["echo", "`a'b`"])),
	("a`b c`d", Some(&["a`b c`d"])),
	("echo `a\\`b`", Some(&["echo", "`a\\`b`"])),
	("echo `` x", Some(&["echo", "``", "x"])),
	("echo `a\"b c`", Some(&["echo", "`a\"b c`"])),
	("echo \"a`b\" c`", None),
];

/// The strings of `TABLE` that carry a `$$` in the file, as the splitter sees
/// them once Compose has interpolated the file: a `$$` is a single `$`. The
/// expected words are what Docker printed, with its re-escaping of `$` undone.
const AFTER_INTERPOLATION: &[(&str, Option<&[&str]>)] = &[
	("echo $(da te)", Some(&["echo", "$(da te)"])),
	("echo $(da te", None),
	("echo $(a;b)", Some(&["echo", "$(a;b)"])),
	("echo $(a (b) c)", None),
	("echo $(a) b", Some(&["echo", "$(a)", "b"])),
	("echo $((1 2))", None),
	("echo $(a\\)b) c", Some(&["echo", "$(a\\)b)", "c"])),
	("echo \"$(a b)\" c", Some(&["echo", "$(a b)", "c"])),
	("echo $(a \"b) c\" d", None),
	("echo $(a 'b) c' d", None),
	("echo a$(b c)d e", Some(&["echo", "a$(b c)d", "e"])),
	("echo $( x", None),
	("echo \"$(a b\" c)", None),
	("echo $(a`b)c`)", None),
	("echo `a$(b`)", None),
	("$(a", None),
	("echo $(a\nb) c", Some(&["echo", "$(a\nb)", "c"])),
	("echo $(a\\\\) b)", None),
	("echo $a b", Some(&["echo", "$a", "b"])),
	("echo $", Some(&["echo", "$"])),
	("echo ${a b}", Some(&["echo", "${a", "b}"])),
];

fn yaml_string(text: &str) -> String {
	serde_json::to_string(text).unwrap()
}

fn load(key: &str, text: &str) -> Result<Service, serde_yaml::Error> {
	serde_yaml::from_str(&format!("image: x\n{key}: {}\n", yaml_string(text)))
}

fn words(service: &Service, key: &str) -> Vec<String> {
	let command = match key {
		"command" => &service.command,
		_ => &service.entrypoint,
	};
	command.as_ref().unwrap().to_container_override()
}

#[test]
fn a_string_command_and_entrypoint_split_like_docker_compose() {
	for key in ["command", "entrypoint"] {
		for (text, expected) in TABLE {
			match (load(key, text), expected) {
				(Ok(service), Some(want)) => {
					assert_eq!(words(&service, key), *want, "{key}: {text:?}");
				}
				(Err(e), None) => {
					let e = e.to_string();
					assert!(e.contains("invalid command line string"), "{key}: {e}");
				}
				(Ok(service), None) => {
					panic!(
						"{key}: {text:?} should be refused, got {:?}",
						words(&service, key)
					)
				}
				(Err(e), Some(_)) => panic!("{key}: {text:?} should load: {e}"),
			}
		}
	}
}

#[test]
fn a_dollar_group_and_a_dollar_split_like_docker_compose() {
	for (text, expected) in AFTER_INTERPOLATION {
		let got = super::split_words(text).ok();
		let want = expected.map(|w| w.iter().map(|s| s.to_string()).collect::<Vec<_>>());
		assert_eq!(got, want, "{text:?}");
	}
}

#[test]
fn a_list_command_is_left_alone() {
	let service: Service =
		serde_yaml::from_str("image: x\ncommand: [\"a b\", \"c;d\", \"\"]\n").unwrap();
	assert_eq!(words(&service, "command"), ["a b", "c;d", ""]);
}

#[test]
fn a_healthcheck_test_string_stays_whole() {
	let service: Service =
		serde_yaml::from_str("image: x\nhealthcheck:\n  test: \"curl -f http://x && y\"\n")
			.unwrap();
	let test = service.healthcheck.unwrap().test.unwrap();
	assert_eq!(test.to_exec(), ["sh", "-c", "curl -f http://x && y"]);
}
