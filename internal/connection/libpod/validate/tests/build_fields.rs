use super::*;

#[test]
fn kv_key_accepts_alnum_dot_dash_underscore() {
	assert!(is_valid_kv_key("FOO"));
	assert!(is_valid_kv_key("foo_bar"));
	assert!(is_valid_kv_key("FOO.BAR"));
	assert!(is_valid_kv_key("FOO-BAR"));
	assert!(is_valid_kv_key("a1b2c3"));
}

#[test]
fn kv_key_rejects_invalid() {
	assert!(!is_valid_kv_key(""));
	assert!(!is_valid_kv_key("FOO BAR"));
	assert!(!is_valid_kv_key("FOO\nBAR"));
	assert!(!is_valid_kv_key("FOO=BAR"));
	assert!(!is_valid_kv_key("FOO;ls"));
}

#[test]
fn pre_validate_build_rejects_bad_key() {
	let mut args = HashMap::new();
	args.insert("MALFORMED\nKEY".to_string(), "value".to_string());
	let labels = HashMap::new();
	let err = pre_validate_build(&args, &labels).unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("build.args"));
	assert!(msg.contains("MALFORMED\\nKEY"));
	assert!(msg.contains("not a valid identifier"));
}

#[test]
fn pre_validate_build_rejects_bad_label() {
	let args = HashMap::new();
	let mut labels = HashMap::new();
	labels.insert("bad=key".to_string(), "value".to_string());
	let err = pre_validate_build(&args, &labels).unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("build.labels"));
	assert!(msg.contains("bad=key"));
	assert!(msg.contains("without `=`, `$` or combining marks"));
}

#[test]
fn label_key_accepts_what_podman_stores_unchanged() {
	for key in [
		"app.kubernetes.io/name",
		"com.example/with:colon@and+plus",
		"a b",
		"a,b;c#d",
		"a\"b",
		"a\\b",
		"a'b`c",
		"{\"a\":1}",
		"\u{f1}and\u{fa}",
		"\u{540d}\u{524d}",
		"a\u{1f600}",
		"\u{e9}",
		"\u{2764}",
	] {
		assert!(is_valid_label_key(key), "{key:?}");
	}
}

#[test]
fn label_key_rejects_what_podman_would_change() {
	for key in [
		"",
		"a=b",
		"a=",
		"=a",
		"a$b",
		"${HOME}",
		"a$",
		"a\nb",
		"a\rb",
		"a\tb",
		"a\0b",
		"a\u{1}b",
		"a\u{b}b",
		"a\u{7f}b",
		"a\u{85}b",
		"a\u{a0}b",
		"a\u{ad}b",
		"a\u{200b}b",
		"a\u{200d}b",
		"a\u{2028}b",
		"a\u{3000}b",
		"a\u{feff}b",
		"a\u{e000}b",
		"a\u{378}b",
		// Podman stores these; refused to keep the check small.
		"e\u{301}",
		"\u{2764}\u{fe0f}",
		// A printable `"` or `\` must not make room for one that is not.
		"\"a\\\nb",
	] {
		assert!(!is_valid_label_key(key), "{key:?}");
	}
}

#[test]
fn pre_validate_build_accepts_a_prefixed_label_key() {
	let args = HashMap::new();
	let mut labels = HashMap::new();
	labels.insert("app.kubernetes.io/name".to_string(), "demo".to_string());
	assert!(pre_validate_build(&args, &labels).is_ok());
}

#[test]
fn pre_validate_build_keeps_the_arg_charset() {
	let mut args = HashMap::new();
	args.insert("app.kubernetes.io/name".to_string(), "demo".to_string());
	let labels: HashMap<String, String> = HashMap::new();
	let err = pre_validate_build(&args, &labels).unwrap_err();
	assert!(err.to_string().contains("build.args"));
}

#[test]
fn pre_validate_build_passes_when_clean() {
	let mut args = HashMap::new();
	args.insert("GOOD".to_string(), "value".to_string());
	let mut labels = HashMap::new();
	labels.insert("GOOD_LABEL".to_string(), "value".to_string());
	assert!(pre_validate_build(&args, &labels).is_ok());
}

#[test]
fn render_value_truncates_long_inputs() {
	let long = "x".repeat(500);
	let r = render_value(&long);
	assert!(r.len() <= 260);
	assert!(r.ends_with('…'));
}

#[test]
fn render_value_escapes_control_chars() {
	assert_eq!(render_value("a\nb"), "a\\nb");
	assert_eq!(render_value("a\tb"), "a\\tb");
	assert_eq!(render_value("a\rb"), "a\\rb");
	assert!(render_value("a\x1bb").contains("\\u{1b}"));
}

#[test]
fn render_value_keeps_normal_strings_intact() {
	assert_eq!(render_value("hello"), "hello");
	assert_eq!(render_value("CAP_NET_ADMIN"), "CAP_NET_ADMIN");
	assert_eq!(render_value("db:10.0.0.2"), "db:10.0.0.2");
}

#[test]
fn build_field_error_uses_empty_service() {
	let e = build_field_error("build.args", "MALFORMED\nKEY", "podman rejected the key");
	match e {
		PodmanError::Field {
			service,
			field,
			value,
			message,
		} => {
			assert_eq!(service, "");
			assert_eq!(field, "build.args");
			assert_eq!(value, "MALFORMED\\nKEY");
			assert_eq!(message, "podman rejected the key");
		}
		_ => panic!("expected Field variant"),
	}
}

#[test]
fn spec_field_error_carries_service() {
	let e = spec_field_error("web", "pid", "evil", "namespace not recognised");
	match e {
		PodmanError::Field {
			service,
			field,
			value,
			message,
		} => {
			assert_eq!(service, "web");
			assert_eq!(field, "pid");
			assert_eq!(value, "evil");
			assert_eq!(message, "namespace not recognised");
		}
		_ => panic!("expected Field variant"),
	}
}
