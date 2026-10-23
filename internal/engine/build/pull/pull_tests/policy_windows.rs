//! Registry comparison and strict interval syntax for time policies.

use super::*;

fn mapped(value: &str) -> &'static str {
	pull_policy_checked(Some(value), "web", None)
		.unwrap()
		.libpod()
}

fn refused(field: &str, value: &str) {
	let result = if field == "pull_policy" {
		pull_policy_checked(Some(value), "web", None)
	} else {
		pull_policy_checked(Some("missing"), "web", Some(value))
	};
	let error = result.unwrap_err();
	assert!(
		matches!(&error, crate::error::ComposeError::Podman(crate::libpod::PodmanError::Field {
		service, field: actual, value: actual_value, ..
	}) if service == "web" && actual == field && actual_value == value)
	);
	let message = error.to_string();
	for accepted in [
		"always",
		"missing",
		"newer",
		"never",
		"if_not_present",
		"build",
		"daily",
		"weekly",
		"refresh",
		"every_",
		"4294967295",
	] {
		assert!(message.contains(accepted), "{message}");
	}
}

#[test]
fn daily_uses_newer() {
	assert_eq!(mapped("daily"), "newer");
}

#[test]
fn weekly_uses_newer() {
	assert_eq!(mapped("weekly"), "newer");
}

#[test]
fn every_uses_newer() {
	assert_eq!(mapped("every_2d"), "newer");
}

#[test]
fn refresh_without_interval_uses_newer() {
	assert_eq!(mapped("refresh"), "newer");
}

#[test]
fn refresh_interval_does_not_change_newer() {
	assert_eq!(
		pull_policy_checked(Some("refresh"), "web", Some("1h30m"))
			.unwrap()
			.libpod(),
		"newer"
	);
}

#[test]
fn intervals_accept_zero_repeated_and_reordered_units() {
	for value in ["0s", "01h1h", "30m1h", "1w2d3h4m5s", "4294967295s"] {
		assert_eq!(mapped(&format!("every_{value}")), "newer");
		assert_eq!(
			pull_policy_checked(Some("missing"), "web", Some(value))
				.unwrap()
				.libpod(),
			"missing"
		);
	}
}

#[test]
fn empty_every_is_refused() {
	refused("pull_policy", "every_");
}

#[test]
fn missing_number_is_refused() {
	refused("pull_policy", "every_h");
}

#[test]
fn unknown_unit_is_refused() {
	refused("pull_policy", "every_5q");
}

#[test]
fn dangling_number_is_refused() {
	refused("pull_policy", "every_1h30");
}

#[test]
fn interval_grammar_is_ascii_and_lowercase() {
	for value in [
		"every_١h",
		"every_１h",
		"every_1H",
		"every_+1h",
		"every_ 1h",
		"every_1h ",
		"every_1.5h",
		"every_500ms",
	] {
		refused("pull_policy", value);
	}
}

#[test]
fn interval_overflow_is_refused() {
	for value in [
		"4294967296s",
		"4294967295s1s",
		"7102w",
		"999999999999999999999999999999s",
	] {
		refused("pull_policy", &format!("every_{value}"));
		refused("pull_refresh_after", value);
	}
}

#[test]
fn refresh_interval_is_validated_with_other_policies() {
	for policy in [
		None,
		Some("missing"),
		Some("always"),
		Some("daily"),
		Some("refresh"),
	] {
		let error = pull_policy_checked(policy, "web", Some("nonsense")).unwrap_err();
		assert!(error.to_string().contains("pull_refresh_after"));
	}
	for value in ["", "h", "1", "500ms", "1.5h", "1H", "١h"] {
		refused("pull_refresh_after", value);
	}
}

#[test]
fn accepted_values_are_in_the_error() {
	refused("pull_policy", "alaways");
}

#[test]
fn service_resolution_uses_newer() {
	let engine = crate::engine::Engine::new(
		crate::libpod::Client::new("/nonexistent.sock"),
		"proj".into(),
	);
	let service = crate::compose::types::Service {
		pull_policy: Some("refresh".into()),
		pull_refresh_after: Some("12h".into()),
		..Default::default()
	};
	assert_eq!(
		engine
			.resolved_pull_policy("web", &service)
			.unwrap()
			.libpod(),
		"newer"
	);
}
