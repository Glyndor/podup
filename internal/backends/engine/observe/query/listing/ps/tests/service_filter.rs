#[cfg(unix)]
use super::engine_with;
#[cfg(unix)]
use crate::compose::types::{ComposeFile, Service};
#[cfg(unix)]
use crate::engine::fake_podman;
#[cfg(unix)]
use crate::engine::query::{ps_rows_for_test, PsFilterOptions};

/// A service scaled at runtime has more replicas than the compose file
/// declares: `up --scale web=3` leaves the daemon with `proj-web-1`,
/// `proj-web-2` and `proj-web-3` while the file still says one replica.
///
/// `ps web` must show all three. The filter used to be built from the
/// compose file's replica count, so the two extra replicas were dropped from
/// the listing even though the daemon had them: the command answered a
/// question about the file when it was asked about the running project.
#[tokio::test]
#[cfg(unix)]
async fn ps_service_filter_shows_the_replicas_a_runtime_scale_created() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-2"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-web-3"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-db-1"],"State":"running","Labels":{"podup.service":"db"}}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());
	file.services.insert("db".into(), Service::default());

	// The compose file declares one replica per service: no `scale:`, no
	// `deploy.replicas`.
	let filters = PsFilterOptions::new(false, vec!["web".to_string()], vec![], vec![]);
	let rows = ps_rows_for_test(&e, &file, &filters)
		.await
		.expect("ps must succeed");
	let names: Vec<&str> = rows
		.iter()
		.map(|r| r["Name"].as_str().unwrap_or(""))
		.collect();

	assert_eq!(
		names.len(),
		3,
		"every replica of the service must be listed: {names:?}"
	);
	for i in 1..=3 {
		assert!(
			names.contains(&format!("proj-web-{i}").as_str()),
			"proj-web-{i} must be listed: {names:?}"
		);
	}
	assert!(
		!names.iter().any(|n| n.contains("db")),
		"another service's container must stay out: {names:?}"
	);
}

/// A container with no service label is matched on its name, the only
/// evidence there is for it. The containers podup creates carry the label, so
/// this is the path for a container that was made by hand.
#[tokio::test]
#[cfg(unix)]
async fn ps_service_filter_falls_back_to_the_name_when_there_is_no_label() {
	let containers = r#"[
		{"Names":["/proj-web-1"],"State":"running","Labels":{"podup.service":"web"}},
		{"Names":["/proj-legacy-1"],"State":"running"}
	]"#;
	let fake = fake_podman::start(move |method, target| {
		if method == "GET" && target.contains("/containers/json") {
			(200, containers.to_string())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = engine_with(fake.client(), "proj");
	let mut file = ComposeFile::default();
	file.services.insert("web".into(), Service::default());

	let filters = PsFilterOptions::new(false, vec!["web".to_string()], vec![], vec![]);
	let rows = ps_rows_for_test(&e, &file, &filters)
		.await
		.expect("ps must succeed");
	let names: Vec<&str> = rows
		.iter()
		.map(|r| r["Name"].as_str().unwrap_or(""))
		.collect();

	assert_eq!(
		names,
		vec!["proj-web-1"],
		"only the service's own containers are listed: {names:?}"
	);
}
