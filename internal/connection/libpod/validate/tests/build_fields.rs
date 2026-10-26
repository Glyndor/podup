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
fn first_invalid_kv_key_returns_first_failure() {
	let keys = ["GOOD", "MALFORMED\nKEY", "ALSO_GOOD"];
	let (field, key, msg) = first_invalid_kv_key("build.args", keys.iter().copied()).unwrap();
	assert_eq!(field, "build.args");
	assert_eq!(key, "MALFORMED\nKEY");
	assert!(msg.contains("not a valid identifier"));
}

#[test]
fn first_invalid_kv_key_passes_when_all_valid() {
	let keys = ["GOOD", "ALSO_GOOD"];
	assert!(first_invalid_kv_key("build.args", keys.iter().copied()).is_none());
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
}

#[test]
fn pre_validate_build_rejects_bad_label() {
	let args = HashMap::new();
	let mut labels = HashMap::new();
	labels.insert("bad key".to_string(), "value".to_string());
	let err = pre_validate_build(&args, &labels).unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("build.labels"));
	assert!(msg.contains("bad key"));
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
