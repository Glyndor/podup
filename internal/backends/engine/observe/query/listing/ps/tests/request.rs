use super::*;

/// `ps --status exited` (or `--filter status=exited`) without `-a` must
/// still find exited containers: libpod's list endpoint only returns
/// running containers when `all=false`, so a status filter must force
/// `all=true` on the outgoing request regardless of `opts.all`.
#[tokio::test]
#[cfg(unix)]
async fn ps_status_filter_forces_all_true_even_when_opts_all_is_false() {
	let fake = fake_podman::start(|method, _target| {
		if method == "GET" {
			(200, "[]".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let file = ComposeFile::default();

	e.ps_filtered(
		&file,
		PsOptions {
			all: false,
			..Default::default()
		},
		PsFilterOptions {
			status: vec!["exited".into()],
			..Default::default()
		},
	)
	.await
	.expect("ps should succeed");

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("/containers/json") && r.contains("all=true")),
		"a status filter must force all=true even with opts.all=false: {seen:?}"
	);
}

/// Without a status filter, `opts.all=false` stays `all=false` on the wire
/// (the common case: `ps` with no flags lists only running containers).
#[tokio::test]
#[cfg(unix)]
async fn ps_without_status_filter_keeps_all_false_by_default() {
	let fake = fake_podman::start(|method, _target| {
		if method == "GET" {
			(200, "[]".to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let file = ComposeFile::default();

	e.ps_filtered(&file, PsOptions::default(), PsFilterOptions::default())
		.await
		.expect("ps should succeed");

	let seen = fake.requests.lock().unwrap();
	assert!(
		seen.iter()
			.any(|r| r.contains("/containers/json") && r.contains("all=false")),
		"no status filter must keep all=false: {seen:?}"
	);
}

/// Service-name output must still query the daemon, using the same all/status
/// widening as the table. Configured services without containers are not rows.
#[tokio::test]
#[cfg(unix)]
async fn ps_services_uses_the_table_container_query() {
	for (all, status, expected_all) in [
		(false, vec![], false),
		(true, vec![], true),
		(false, vec!["exited".into()], true),
	] {
		let fake = fake_podman::start(|_, _| (200, "[]".into()));
		let engine = engine_with(fake.client(), "proj");
		let file = crate::compose::parse_str("services:\n  web: {image: alpine}\n").unwrap();
		engine
			.ps_filtered(
				&file,
				PsOptions::new(all, false, false),
				PsFilterOptions::new(true, vec![], status, vec![]),
			)
			.await
			.unwrap();
		let seen = fake.requests.lock().unwrap();
		assert!(
			seen.iter()
				.any(|r| r.contains("/containers/json")
					&& r.contains(&format!("all={expected_all}"))),
			"{seen:?}"
		);
	}
}
