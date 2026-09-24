//! Tests for `build_log_config`, the bridge between compose `logging:` and
//! libpod's `LogConfig`. Split from `mod.rs` so the production file stays under
//! the 500-line cap and each surface (struct vs wire JSON vs malformed input
//! vs `max-file` drop) gets its own focused case (#1417).

use super::*;
use crate::compose::types::LoggingConfig;

#[test]
fn log_config_applies_the_default_when_absent() {
	// A service with no `logging:` block gets the rotation default so
	// containers cannot run with unbounded log growth (#1354). libpod
	// reads rotation from `size`, not from options.max-size, so the
	// default must travel in the typed field (#1417).
	let cfg = build_log_config("web", None)
		.expect("absent -> default, not error")
		.expect("absent -> default, not None");
	assert_eq!(cfg.driver.as_deref(), Some("k8s-file"));
	assert_eq!(cfg.size, Some(10 * 1024 * 1024));
	// `max-size`/`max-file` no longer travel inside options; libpod would
	// ignore them there, leaving the container with -1B.
	assert!(!cfg.options.contains_key("max-size"));
	assert!(!cfg.options.contains_key("max-file"));
}

#[test]
fn log_config_default_serializes_size_as_bytes() {
	// Struct-level assertions passed before #1417 was filed and the
	// defect still shipped. The wire format is what libpod parses, so the
	// fix has to be anchored on the serialized JSON.
	let cfg = build_log_config("web", None).unwrap().unwrap();
	let v = serde_json::to_value(&cfg).unwrap();
	assert_eq!(v["driver"], "k8s-file");
	assert_eq!(v["size"], 10 * 1024 * 1024);
	assert!(
		v.get("options").is_none(),
		"empty options must be elided, got {v}"
	);
}

#[test]
fn log_config_driver_only() {
	let logging = LoggingConfig {
		driver: Some("json-file".into()),
		options: Default::default(),
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(cfg.driver.as_deref(), Some("json-file"));
	assert!(cfg.size.is_none());
	assert!(cfg.options.is_empty());
}

#[test]
fn log_config_with_max_size_parses_into_typed_field() {
	let mut opts = std::collections::HashMap::new();
	opts.insert("max-size".into(), "10m".into());
	let logging = LoggingConfig {
		driver: Some("json-file".into()),
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(cfg.size, Some(10 * 1024 * 1024));
	// And on the wire, what libpod actually parses, the key is `size`
	// and the value is a number, not the suffixed string under options.
	let v = serde_json::to_value(&cfg).unwrap();
	assert_eq!(v["size"], 10 * 1024 * 1024);
	assert_eq!(v["size"].as_i64().unwrap(), 10_485_760);
	assert!(
		v["options"].get("max-size").is_none(),
		"max-size must not also travel in options: {v}"
	);
}

#[test]
fn log_config_max_size_minus_one_disables_rotation() {
	// libpod treats `size: -1` as unlimited, mirroring Docker's
	// `--log-opt max-size=-1` convention. Compose users reach for the
	// same string, so the parser must surface it rather than drop it.
	let mut opts = std::collections::HashMap::new();
	opts.insert("max-size".into(), "-1".into());
	let logging = LoggingConfig {
		driver: Some("json-file".into()),
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(cfg.size, Some(-1));
	let v = serde_json::to_value(&cfg).unwrap();
	assert_eq!(v["size"], -1);
}

#[test]
fn log_config_max_size_plain_bytes() {
	let mut opts = std::collections::HashMap::new();
	opts.insert("max-size".into(), "1048576".into());
	let logging = LoggingConfig {
		driver: Some("json-file".into()),
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(cfg.size, Some(1_048_576));
}

#[test]
fn log_config_max_file_is_dropped_with_warning() {
	// libpod does not implement `max-file` on any path; forwarding it as
	// a `LogOpt` would silently disappear. The unit test asserts the
	// struct shape; the warning side is exercised by the engine path
	// and surfaced separately.
	let mut opts = std::collections::HashMap::new();
	opts.insert("max-size".into(), "10m".into());
	opts.insert("max-file".into(), "5".into());
	let logging = LoggingConfig {
		driver: Some("json-file".into()),
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(cfg.size, Some(10 * 1024 * 1024));
	assert!(
		!cfg.options.contains_key("max-file"),
		"max-file must be dropped, got {v}",
		v = serde_json::to_string(&cfg).unwrap()
	);
	// And nothing carrying the dead key reached the wire.
	let v = serde_json::to_value(&cfg).unwrap();
	assert!(
		v.get("options").is_none(),
		"options leaked onto the wire: {v}"
	);
}

#[test]
fn log_config_max_size_malformed_returns_field_error() {
	// libpod would answer this with a 500 ("cannot unmarshal string into
	// size of type int64"); surfacing it as a PodmanError::Field points
	// the user at the compose key they wrote, not at libpod's Go type
	// (#1417).
	let mut opts = std::collections::HashMap::new();
	opts.insert("max-size".into(), "diez megas".into());
	let logging = LoggingConfig {
		driver: Some("json-file".into()),
		options: opts,
	};
	let err = build_log_config("web", Some(&logging)).unwrap_err();
	match err {
		ComposeError::Podman(crate::libpod::error::PodmanError::Field {
			service,
			field,
			value,
			..
		}) => {
			assert_eq!(service, "web");
			assert_eq!(field, "logging.options.max-size");
			assert_eq!(value, "diez megas");
		}
		other => panic!("expected PodmanError::Field, got {other:?}"),
	}
}

#[test]
fn log_config_unsupported_options_pass_through() {
	// `path` and `tag` are the only options libpod honours for the
	// built-in drivers (see `man podman-run`); they must reach the wire
	// unchanged when the user sets them.
	let mut opts = std::collections::HashMap::new();
	opts.insert("path".into(), "/var/log/app.log".into());
	opts.insert("tag".into(), "{{.Name}}".into());
	let logging = LoggingConfig {
		driver: Some("json-file".into()),
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	let v = serde_json::to_value(&cfg).unwrap();
	assert_eq!(v["options"]["path"], "/var/log/app.log");
	assert_eq!(v["options"]["tag"], "{{.Name}}");
}

#[test]
fn log_config_options_without_driver_keep_the_default_driver() {
	// A `logging:` block that names only options used to land in libpod
	// with `driver=None`, which let the containers.conf driver shadow the
	// `k8s-file` default podup applies (#1895). The user-supplied
	// `max-size` still wins on `size`.
	let mut opts = std::collections::HashMap::new();
	opts.insert("max-size".into(), "1m".into());
	let logging = LoggingConfig {
		driver: None,
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(cfg.driver.as_deref(), Some("k8s-file"));
	assert_eq!(cfg.size, Some(1_048_576));
}

#[test]
fn log_config_options_without_driver_or_size_are_left_to_the_host() {
	// A driverless `logging:` block with options that are NOT `max-size`
	// (here just `tag`) must leave both `driver` and `size` unset so the
	// host's containers.conf default applies unchanged; podup no longer
	// injects `k8s-file` and a 10m cap here, which used to shadow a
	// journald host config (#1895).
	let mut opts = std::collections::HashMap::new();
	opts.insert("tag".into(), "myapp".into());
	let logging = LoggingConfig {
		driver: None,
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(cfg.driver, None);
	assert_eq!(cfg.size, None);
	assert_eq!(
		cfg.options.get("tag").map(String::as_str),
		Some("myapp"),
		"tag must still pass through to libpod: {cfg:?}"
	);
}

#[test]
fn log_config_named_driver_without_size_stays_uncapped() {
	// The default size only kicks in when the driver is absent; a named
	// driver without `max-size` still produces `size=None`, matching the
	// behaviour before #1895.
	let logging = LoggingConfig {
		driver: Some("k8s-file".into()),
		options: Default::default(),
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(cfg.driver.as_deref(), Some("k8s-file"));
	assert_eq!(cfg.size, None);
}

#[test]
fn log_config_driverless_minus_one_size_keeps_host_driver() {
	// Regression: a driverless `logging:` block whose `max-size` resolves to
	// `-1` must NOT fall back to `k8s-file`. The fallback forces a file
	// driver with no cap, but the user wrote `-1` to opt out of capping;
	// the previous behaviour silently stripped a journald host driver
	// (#1895 follow-up).
	//
	// `size::parse_memory` has a special case for the literal `"-1"` (the
	// "unlimited" sentinel), returning `Some(-1)`. Lock the parser output
	// down first, then assert the produced `size` matches it, so a future
	// change to `parse_memory` cannot silently shift what this regression
	// covers.
	let parsed = crate::size::parse_memory("-1");
	assert_eq!(parsed, Some(-1), "`-1` is the unlimited sentinel");

	let mut opts = std::collections::HashMap::new();
	opts.insert("max-size".into(), "-1".into());
	opts.insert("tag".into(), "myapp".into());
	let logging = LoggingConfig {
		driver: None,
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(
		cfg.driver, None,
		"driverless `-1` size must not pin k8s-file: {cfg:?}"
	);
	assert_eq!(
		cfg.size, parsed,
		"size field must mirror what parse_memory produces for `-1`: {cfg:?}"
	);
	assert_eq!(
		cfg.options.get("tag").map(String::as_str),
		Some("myapp"),
		"non-size options must still pass through to libpod: {cfg:?}"
	);
}

#[test]
fn log_config_driverless_zero_size_keeps_host_driver() {
	// Regression: a driverless `logging:` block whose `max-size` resolves to
	// `0` must NOT fall back to `k8s-file`. Zero is not a positive size
	// (no cap to apply) and only `k8s-file` would read the typed field,
	// so the fallback would strip the host driver for no rotation benefit
	// (#1895 follow-up).
	//
	// `size::parse_memory` accepts a bare `0` and produces `Some(0)`; assert
	// that the parser result is what gets surfaced on the wire so any future
	// change to `parse_memory` either keeps this regression valid or breaks
	// it loudly.
	let parsed = crate::size::parse_memory("0");
	assert_eq!(parsed, Some(0), "bare `0` parses to a zero byte count");

	let mut opts = std::collections::HashMap::new();
	opts.insert("max-size".into(), "0".into());
	opts.insert("tag".into(), "myapp".into());
	let logging = LoggingConfig {
		driver: None,
		options: opts,
	};
	let cfg = build_log_config("web", Some(&logging)).unwrap().unwrap();
	assert_eq!(
		cfg.driver, None,
		"driverless `0` size must not pin k8s-file: {cfg:?}"
	);
	assert_eq!(
		cfg.size, parsed,
		"size field must mirror what parse_memory produces for `0`: {cfg:?}"
	);
	assert_eq!(
		cfg.options.get("tag").map(String::as_str),
		Some("myapp"),
		"non-size options must still pass through to libpod: {cfg:?}"
	);
}
