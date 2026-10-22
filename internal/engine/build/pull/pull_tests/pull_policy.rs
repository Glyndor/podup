use super::*;

#[test]
fn pull_policy_maps_every_spec_value() {
	assert_eq!(libpod_pull_policy(Some("always")), Some("always"));
	assert_eq!(libpod_pull_policy(Some("newer")), Some("newer"));
	assert_eq!(libpod_pull_policy(Some("never")), Some("never"));
	assert_eq!(libpod_pull_policy(Some("missing")), Some("missing"));
	// `if_not_present` is the spec alias for `missing`.
	assert_eq!(libpod_pull_policy(Some("if_not_present")), Some("missing"));
	assert_eq!(libpod_pull_policy(Some("build")), Some("missing"));
	assert_eq!(libpod_pull_policy(None), Some("missing"));
	// Unknown values are reported (None) so the caller fails loud (#1369).
	assert_eq!(libpod_pull_policy(Some("bogus")), None);
}

/// A typo'd `pull_policy:` must surface as a hard error (#1369): the
/// previous warn-and-default-to-missing path silently turned `alaways`
/// into the opposite of what the user wrote, and a `never` typo pulled
/// fresh images on every `up` without telling the operator.
#[test]
fn resolved_pull_policy_rejects_an_unknown_value() {
	let e = crate::engine::Engine::new(
		crate::libpod::Client::new("/nonexistent.sock"),
		"proj".into(),
	);
	let svc = crate::compose::types::Service {
		image: Some("nginx:1.27".to_string()),
		pull_policy: Some("alaways".to_string()),
		..crate::compose::types::Service::default()
	};
	let err = e.resolved_pull_policy("web", &svc).unwrap_err();
	let msg = err.to_string();
	assert!(msg.contains("alaways"), "got {msg}");
	assert!(msg.contains("always"), "got {msg}");
	assert!(
		matches!(
			err,
			crate::error::ComposeError::Podman(crate::libpod::PodmanError::Field {
				ref service,
				ref field,
				..
			}) if service == "web" && field == "pull_policy"
		),
		"unknown pull policy must surface as a Field error carrying the service and field name, got {err:?}"
	);
}

/// #1656: `x-podman-autoupdate: registry` resolves to `newer` on every `up`,
/// with no `--pull` override set. The extension's whole point is to check
/// the registry, so a service that opts in must not silently fall back to
/// `missing` and skip the registry pull.
#[test]
fn x_podman_autoupdate_registry_pulls_newer_on_up() {
	let e = crate::engine::Engine::new(
		crate::libpod::Client::new("/nonexistent.sock"),
		"proj".into(),
	);
	let svc = crate::parse_str(
		"services:\n  web:\n    image: nginx:1.27\n    x-podman-autoupdate: registry\n",
	)
	.unwrap()
	.services
	.swap_remove("web")
	.unwrap();
	assert_eq!(
		e.resolved_pull_policy("web", &svc).unwrap(),
		"newer",
		"x-podman-autoupdate: registry must force newer when no --pull override is in play"
	);
}

/// `--pull` on the command line wins over the extension's `registry` value,
/// because that is what `--pull` is for. Without that override the extension
/// would silently override an operator who wanted `always` or `missing`.
#[test]
fn x_podman_autoupdate_yields_to_an_explicit_pull_flag() {
	let e = crate::engine::Engine::new(
		crate::libpod::Client::new("/nonexistent.sock"),
		"proj".into(),
	)
	.with_up_overrides(Some("missing".to_string()), false, false);
	let svc = crate::parse_str(
		"services:\n  web:\n    image: nginx:1.27\n    x-podman-autoupdate: registry\n",
	)
	.unwrap()
	.services
	.swap_remove("web")
	.unwrap();
	assert_eq!(
		e.resolved_pull_policy("web", &svc).unwrap(),
		"missing",
		"--pull must override the x-podman-autoupdate extension"
	);

	let e = crate::engine::Engine::new(
		crate::libpod::Client::new("/nonexistent.sock"),
		"proj".into(),
	)
	.with_up_overrides(Some("always".to_string()), false, false);
	assert_eq!(
		e.resolved_pull_policy("web", &svc).unwrap(),
		"always",
		"--pull always must still win"
	);
}

/// `x-podman-autoupdate: local` is an opt-out of the registry check, the
/// service's own `pull_policy:` is honoured unchanged.
#[test]
fn x_podman_autoupdate_local_leaves_the_pull_policy_alone() {
	let e = crate::engine::Engine::new(
		crate::libpod::Client::new("/nonexistent.sock"),
		"proj".into(),
	);
	let svc = crate::parse_str(
		"services:\n  web:\n    image: nginx:1.27\n    x-podman-autoupdate: local\n",
	)
	.unwrap()
	.services
	.swap_remove("web")
	.unwrap();
	assert_eq!(
		e.resolved_pull_policy("web", &svc).unwrap(),
		"missing",
		"local: no extension override, default pull_policy (missing) stays"
	);

	let svc = crate::parse_str(
		"services:\n  web:\n    image: nginx:1.27\n    pull_policy: always\n    x-podman-autoupdate: local\n",
	)
	.unwrap()
	.services
	.swap_remove("web")
	.unwrap();
	assert_eq!(
		e.resolved_pull_policy("web", &svc).unwrap(),
		"always",
		"local: an explicit pull_policy: must survive untouched"
	);
}
