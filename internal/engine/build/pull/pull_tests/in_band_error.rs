use super::*;

/// #1076: libpod reports a failed pull as an in-band `error` line on a 200
/// response. That line used to be warned about and dropped, so every caller
/// believed the pull had succeeded.
///
/// The image is present here (a stale copy from an earlier pull) which is
/// exactly the case a presence probe cannot catch: it passes while the pull
/// failed. `up --pull always` against an unreachable registry therefore
/// started yesterday's image and exited 0.
#[tokio::test]
#[cfg(unix)]
async fn a_failed_pull_is_reported_even_when_a_stale_image_is_present() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/images/pull") {
			// 200 with an in-band error, the way libpod reports it.
			(
				200,
				r#"{"error":"initializing source: pinging container registry: connection refused"}"#
					.to_string(),
			)
		} else if method == "GET" && target.contains("/images/") && target.contains("/json") {
			// The stale image is in local storage.
			(200, "{}".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::new(fake.client(), "proj".into());
	let file = crate::parse_str("services:\n  a:\n    image: stale:v1\n").unwrap();

	let err = e
		.pull_services(&file, &[])
		.await
		.expect_err("a pull that libpod reported as failed must not be reported as success");
	assert!(
		format!("{err}").contains("connection refused"),
		"the underlying cause must survive: {err}"
	);
}

/// The escape hatch still works: `--ignore-pull-failures` is deliberately
/// exit-0, and that must not change just because the failure is now visible.
#[tokio::test]
#[cfg(unix)]
async fn ignore_pull_failures_still_exits_zero_on_an_in_band_error() {
	let fake = fake_podman::start(|method, target| {
		if method == "POST" && target.contains("/images/pull") {
			(200, r#"{"error":"connection refused"}"#.to_string())
		} else if method == "GET" && target.contains("/images/") && target.contains("/json") {
			(200, "{}".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = crate::engine::Engine::new(fake.client(), "proj".into());
	let file = crate::parse_str("services:\n  a:\n    image: stale:v1\n").unwrap();

	e.pull_services_with_options(
		&file,
		&[],
		crate::engine::PullOptions {
			ignore_failures: true,
			..Default::default()
		},
	)
	.await
	.expect("--ignore-pull-failures must stay exit 0");
}
