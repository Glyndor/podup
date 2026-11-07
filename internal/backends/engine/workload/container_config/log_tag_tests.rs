//! `logging.options.tag` with the drivers Podman cannot give a tag.
//!
//! Measured on Podman 5.7.0, with its own CLI as well: `--log-driver json-file
//! --log-opt tag=x` ends in `k8s-file doesn't support --log-tag` and `conmon
//! failed`, and the container does not start. Docker accepts the tag.

use super::*;
use crate::compose::types::LoggingConfig;

fn config(driver: Option<&str>, options: &[(&str, &str)]) -> LogConfig {
	let logging = LoggingConfig {
		driver: driver.map(str::to_string),
		options: options
			.iter()
			.map(|(k, v)| ((*k).to_string(), (*v).to_string()))
			.collect(),
	};
	build_log_config("web", Some(&logging)).unwrap().unwrap()
}

#[test]
fn tag_is_dropped_for_json_file_and_k8s_file() {
	for driver in ["json-file", "k8s-file"] {
		let cfg = config(Some(driver), &[("tag", "mytag"), ("env", "a")]);
		assert!(!cfg.options.contains_key("tag"), "{driver}");
		assert_eq!(
			cfg.options.get("env").map(String::as_str),
			Some("a"),
			"the rest stays"
		);
	}
}

#[test]
fn tag_is_dropped_when_max_size_pins_the_default_driver() {
	// No `driver:` and a positive `max-size`: the k8s-file default is pinned
	// (#1895), so the tag would reach the logger that refuses it.
	let cfg = config(None, &[("tag", "mytag"), ("max-size", "10m")]);
	assert_eq!(cfg.driver.as_deref(), Some("k8s-file"));
	assert!(!cfg.options.contains_key("tag"));
}

#[test]
fn tag_is_kept_for_the_other_drivers_and_when_the_host_decides_the_driver() {
	for driver in ["journald", "syslog", "fluentd"] {
		let cfg = config(Some(driver), &[("tag", "mytag")]);
		assert_eq!(
			cfg.options.get("tag").map(String::as_str),
			Some("mytag"),
			"{driver}"
		);
	}
	// No driver and no size cap: the host's containers.conf picks the driver, and
	// which it is cannot be known here.
	let cfg = config(None, &[("tag", "mytag")]);
	assert_eq!(cfg.options.get("tag").map(String::as_str), Some("mytag"));
}
