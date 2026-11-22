use super::{render_healthcheck, Section};
use crate::compose::types::Service;

/// Render a `[Container]` section for a service parsed from `yaml`,
/// returning `(rendered_body, warnings)`.
fn render(yaml: &str) -> (String, Vec<String>) {
	let service: Service = serde_yaml::from_str(yaml).expect("service parses");
	let mut container = Section::new("Container");
	let mut warnings = Vec::new();
	let _ = render_healthcheck("web", &service, &mut container, &mut warnings);
	(container.render(), warnings)
}

#[test]
fn no_healthcheck_emits_nothing() {
	let (body, warnings) = render("image: x\n");
	assert_eq!(body, "[Container]\n");
	assert!(warnings.is_empty());
}

#[test]
fn disabled_healthcheck_emits_none() {
	let (body, warnings) = render("image: x\nhealthcheck:\n  disable: true\n");
	assert!(body.contains("HealthCmd=none"));
	// A disabled check short-circuits, so no timing keys follow.
	assert!(!body.contains("HealthInterval"));
	assert!(warnings.is_empty());
}

#[test]
fn exec_test_strips_cmd_sentinel_and_renders_timing() {
	let (body, warnings) = render(
		"image: x\nhealthcheck:\n  test: [\"CMD\", \"curl\", \"-f\", \"http://localhost\"]\n  \
		 interval: 30s\n  timeout: 5s\n  retries: 3\n  start_period: 10s\n",
	);
	// The exec form is written as a JSON array, which Podman keeps as an exec test.
	assert!(body.contains("HealthCmd=[\"CMD\",\"curl\",\"-f\",\"http://localhost\"]\n"));
	assert!(body.contains("HealthInterval=30s"));
	assert!(body.contains("HealthTimeout=5s"));
	assert!(body.contains("HealthRetries=3"));
	assert!(body.contains("HealthStartPeriod=10s"));
	// The exec form runs without a shell, so nothing is reported.
	assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn shell_test_renders_as_is() {
	let (body, _) = render("image: x\nhealthcheck:\n  test: curl -f http://localhost\n");
	assert!(body.contains("HealthCmd=curl -f http://localhost"));
}

#[test]
fn exec_test_without_sentinel_keeps_all_parts() {
	// An exec array whose first element is not CMD/CMD-SHELL/NONE is rendered
	// verbatim (no sentinel stripped).
	let (body, _) = render("image: x\nhealthcheck:\n  test: [\"pg_isready\", \"-q\"]\n");
	assert!(body.contains("HealthCmd=pg_isready -q"));
}

#[test]
fn start_interval_is_skipped_with_warning() {
	let (body, warnings) =
		render("image: x\nhealthcheck:\n  test: [\"CMD\", \"true\"]\n  start_interval: 5s\n");
	assert!(body.contains("HealthCmd=[\"CMD\",\"true\"]"));
	// No key is emitted for start_interval, but the user is warned.
	assert!(
		warnings.iter().any(|w| w.contains("start_interval")),
		"{warnings:?}"
	);
}

/// Without this systemd calls the unit started as soon as the container is
/// created, so `After=`/`Requires=` order creation rather than readiness.
/// The 6.2.0 generator turns it into `--sdnotify=healthy`, so a dependant
/// starts once the probe has passed instead of when the container is created.
#[test]
fn a_healthcheck_brings_notify_healthy() {
	let (body, _) = render("image: x\nhealthcheck:\n  test: [\"CMD\", \"true\"]\n");
	assert!(body.contains("Notify=healthy"), "{body}");
}

/// `--sdnotify=healthy` with nothing to probe would leave the unit starting
/// until `TimeoutStartSec` expires, so it must never be emitted alone.
#[test]
fn no_healthcheck_means_no_notify() {
	let (body, _) = render("image: x\n");
	assert!(!body.contains("Notify"), "{body}");
}

#[test]
fn a_disabled_healthcheck_means_no_notify() {
	let (body, _) = render("image: x\nhealthcheck:\n  disable: true\n");
	assert!(!body.contains("Notify"), "{body}");
}

/// An empty command emits no `HealthCmd`, so there is nothing for systemd
/// to wait on and `Notify` must not appear either.
#[test]
fn an_empty_test_means_no_notify() {
	let (body, _) = render("image: x\nhealthcheck:\n  test: []\n");
	assert!(!body.contains("Notify"), "{body}");
}

/// A plain string `HealthCmd=` is one shell command line, so joining the
/// vector `["CMD", "sh", "-c", "wget -q x || exit 1"]` with spaces would run
/// `wget` with no argument and then `exit 1`. The vector is written as a JSON
/// array instead, which keeps every word as the file said it.
#[test]
fn an_exec_form_healthcheck_keeps_the_words_it_was_given() {
	let (body, _) = render(
		"image: x\nhealthcheck:\n  test: [\"CMD\", \"sh\", \"-c\", \"wget -q x || exit 1\"]\n",
	);
	assert!(
		body.contains("HealthCmd=[\"CMD\",\"sh\",\"-c\",\"wget -q x || exit 1\"]\n"),
		"{body}"
	);
}

#[test]
fn a_shell_form_healthcheck_is_written_as_it_is() {
	let (body, _) =
		render("image: x\nhealthcheck:\n  test: [\"CMD-SHELL\", \"wget -q x || exit 1\"]\n");
	assert!(body.contains("HealthCmd=wget -q x || exit 1"), "{body}");
}

#[test]
fn a_quote_inside_an_exec_word_stays_in_its_word() {
	let (body, _) = render("image: x\nhealthcheck:\n  test: [\"CMD\", \"echo\", \"it's\"]\n");
	assert!(
		body.contains("HealthCmd=[\"CMD\",\"echo\",\"it's\"]\n"),
		"{body}"
	);
}

/// A `CMD-SHELL` or plain string test already runs through a shell, so it is
/// not reported.
#[test]
fn a_shell_test_is_not_reported() {
	for test in ["[\"CMD-SHELL\", \"curl -f x\"]", "curl -f x"] {
		let (_, warnings) = render(&format!("image: x\nhealthcheck:\n  test: {test}\n"));
		assert!(warnings.is_empty(), "{test}: {warnings:?}");
	}
}
