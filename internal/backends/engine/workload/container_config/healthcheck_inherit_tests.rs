//! A compose healthcheck that leaves values out takes them from the image's.

use super::*;
use crate::compose::types::{Command as ComposeCommand, HealthCheck};
use crate::libpod::types::image::ImageHealthcheck;

const S: i64 = 1_000_000_000;

/// `HEALTHCHECK --interval=1m --timeout=5s --retries=7 --start-period=3s CMD /bin/true`.
fn image() -> ImageHealthcheck {
	ImageHealthcheck {
		test: Some(vec!["CMD-SHELL".into(), "/bin/true".into()]),
		interval: Some(60 * S),
		timeout: Some(5 * S),
		start_period: Some(3 * S),
		start_interval: None,
		retries: Some(7),
	}
}

fn test_only() -> HealthCheck {
	HealthCheck {
		test: Some(ComposeCommand::Exec(vec!["CMD".into(), "true".into()])),
		..Default::default()
	}
}

#[test]
fn a_test_alone_overrides_the_test_and_keeps_the_timings_of_the_image() {
	let cfg = build_healthcheck_inheriting(&test_only(), Some(&image())).unwrap();
	assert_eq!(cfg.test.unwrap(), vec!["CMD", "true"]);
	assert_eq!(cfg.interval, Some(60 * S));
	assert_eq!(cfg.timeout, Some(5 * S));
	assert_eq!(cfg.retries, Some(7));
	assert_eq!(cfg.start_period, Some(3 * S));
}

#[test]
fn a_value_the_compose_file_sets_wins_over_the_image() {
	let hc = HealthCheck {
		interval: Some("10s".into()),
		retries: Some(2),
		..test_only()
	};
	let cfg = build_healthcheck_inheriting(&hc, Some(&image())).unwrap();
	assert_eq!(cfg.interval, Some(10 * S));
	assert_eq!(cfg.retries, Some(2));
	assert_eq!(
		cfg.timeout,
		Some(5 * S),
		"the one it left out is the image's"
	);
}

#[test]
fn what_neither_the_file_nor_the_image_gives_is_the_specification_default() {
	let bare = ImageHealthcheck {
		test: Some(vec!["CMD".into(), "x".into()]),
		..Default::default()
	};
	let cfg = build_healthcheck_inheriting(&test_only(), Some(&bare)).unwrap();
	assert_eq!(cfg.interval, Some(30 * S));
	assert_eq!(cfg.timeout, Some(30 * S));
	assert_eq!(cfg.retries, Some(3));
	assert_eq!(cfg.start_period, None);
}

#[test]
fn an_image_without_a_healthcheck_leaves_the_old_behaviour() {
	let with_none = build_healthcheck_inheriting(&test_only(), None).unwrap();
	let plain = build_healthcheck(&test_only()).unwrap();
	assert_eq!(with_none.interval, plain.interval);
	assert_eq!(with_none.timeout, Some(30 * S));
	assert_eq!(with_none.retries, Some(3));
}

#[test]
fn timings_without_a_test_bring_the_image_test_so_the_probe_survives() {
	let hc = HealthCheck {
		interval: Some("10s".into()),
		..Default::default()
	};
	let cfg = build_healthcheck_inheriting(&hc, Some(&image())).unwrap();
	assert_eq!(cfg.test.unwrap(), vec!["CMD-SHELL", "/bin/true"]);
	assert_eq!(cfg.interval, Some(10 * S));
	assert_eq!(cfg.retries, Some(7));
}

#[test]
fn an_image_that_disables_its_healthcheck_lends_no_test() {
	let off = ImageHealthcheck {
		test: Some(vec!["NONE".into()]),
		..image()
	};
	let hc = HealthCheck {
		interval: Some("10s".into()),
		..Default::default()
	};
	let cfg = build_healthcheck_inheriting(&hc, Some(&off)).unwrap();
	assert!(cfg.test.is_none());
}

#[test]
fn nothing_set_in_the_file_still_sends_no_config_whatever_the_image_says() {
	assert!(build_healthcheck_inheriting(&HealthCheck::default(), Some(&image())).is_none());
}

#[test]
fn a_disabled_healthcheck_stays_disabled() {
	let hc = HealthCheck {
		disable: Some(true),
		..Default::default()
	};
	let cfg = build_healthcheck_inheriting(&hc, Some(&image())).unwrap();
	assert_eq!(cfg.test.unwrap(), vec!["NONE"]);
	assert!(cfg.interval.is_none());
}
