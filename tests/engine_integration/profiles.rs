//! Engine integration tests for profile activation: the `COMPOSE_PROFILES`
//! environment variable, and the dependency whose profile is not active.
use super::*;
// ---------------------------------------------------------------------------
// Profiles: COMPOSE_PROFILES env var path
// ---------------------------------------------------------------------------

#[test]
fn active_profiles_via_env() {
	let rt = tokio::runtime::Runtime::new().unwrap();
	// Set COMPOSE_PROFILES so active_profiles_set reads it (covers profiles.rs L15-19)
	temp_env::with_var("COMPOSE_PROFILES", Some("prod"), || {
		rt.block_on(async {
			let client = match podman().await {
				Some(d) => d,
				None => return,
			};
			let proj = proj("apv");
			let engine = Engine::new(client, proj.clone());
			// "debug" service has profile "debug", not in "prod" → skipped
			let file = parse_str(
				"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n  debug:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    profiles: [\"debug\"]\n",
			)
			.unwrap();
			// Pass empty active_profiles slice so it falls back to COMPOSE_PROFILES env
			engine
				.up_with_options(&file, false, &[], &[], false, false, false, false)
				.await
				.unwrap();
			let mut names = engine
				.test_project_container_names()
				.await
				.unwrap_or_default();
			names.sort();
			engine.down(&file).await.unwrap();

			// COMPOSE_PROFILES=prod must not enable the "debug" profile. Reading it
			// and then starting everything anyway returned Ok just as happily.
			assert_eq!(
				names,
				vec![format!("{proj}-web-1")],
				"COMPOSE_PROFILES=prod started the debug-profiled service"
			);
		});
	});
}

#[tokio::test]
async fn dep_on_profile_filtered_service() {
	let client = match podman().await {
		Some(d) => d,
		None => return,
	};
	let proj = proj("dpf");
	let engine = Engine::new(client, proj.clone());
	// podup ACTIVATES a profile-filtered service when a service that is running
	// depends on it, transitively, so a retained service never points at a dropped
	// one. The rationale is in internal/engine/project/profiles.rs. This comment used to
	// say the opposite (that db is skipped and its dep wait skipped with it)
	// which is what a deliberate design looks like when nothing pins it: the next
	// reader believes the comment over the code.
	let file = parse_str(
		"services:\n  db:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    profiles: [\"debug\"]\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n    depends_on:\n      - db\n",
	)
	.unwrap();

	engine
		.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.unwrap();
	let mut names = engine
		.test_project_container_names()
		.await
		.unwrap_or_default();
	names.sort();
	engine.down(&file).await.unwrap();

	// This is a DELIBERATE divergence from docker compose, pinned here so it
	// cannot be reverted by accident. Measured 2026-08-02 against docker-compose
	// v5.1.3 on the same Podman socket: it refuses the project with
	// `service "web" depends on undefined service "db"`, a misleading message,
	// since db is defined and filtered, not undefined.
	//
	// podup is not departing from a standard. The Compose Specification says only
	// that a profiled service starts "if the profile is activated" and is silent
	// on a dependency whose profile is inactive; Docker's own documentation makes
	// it a requirement on the author (same profile, started separately, or
	// unprofiled) rather than a rule the spec imposes. So the reference errors on
	// a situation that is satisfiable, and podup resolves it.
	//
	// If you are here because this looks wrong: read #1276 and
	// docs/docker-migration.md first. Changing it is a behaviour change and a
	// breaking one, not a bug fix.
	assert_eq!(
		names,
		vec![format!("{proj}-db-1"), format!("{proj}-web-1")],
		"the profile-gated dependency was not activated; this divergence is deliberate, see #1276"
	);
}
