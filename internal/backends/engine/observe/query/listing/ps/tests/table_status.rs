use super::*;

fn entry_exit(state: &str, code: Option<i32>) -> ContainerListEntry {
	ContainerListEntry {
		image_id: String::new(),
		exit_code: code,
		..entry("", state)
	}
}

#[test]
fn table_status_shows_exit_code_for_bare_exited() {
	// A crash (non-zero) and a clean exit (zero) must be distinguishable,
	// even though libpod reports both as a bare `exited` state.
	assert_eq!(
		table_status(&entry_exit("exited", Some(7)), NOW),
		"Exited (7)"
	);
	assert_eq!(
		table_status(&entry_exit("exited", Some(0)), NOW),
		"Exited (0)"
	);
	// Missing exit code defaults to 0 rather than rendering a bare word.
	assert_eq!(table_status(&entry_exit("exited", None), NOW), "Exited (0)");
	// `dead` is treated like an exit too.
	assert_eq!(
		table_status(&entry_exit("dead", Some(255)), NOW),
		"Exited (255)"
	);
}

#[test]
fn table_status_keeps_running_and_rich_status_text() {
	assert_eq!(
		table_status(&entry("Up 2 seconds", "running"), NOW),
		"Up 2 seconds"
	);
	assert_eq!(table_status(&entry("", "running"), NOW), "running");
	// A Docker-style status that already carries the code is left untouched.
	let c = ContainerListEntry {
		image_id: String::new(),
		exit_code: Some(7),
		..entry("Exited (7) 4 seconds ago", "exited")
	};
	assert_eq!(table_status(&c, NOW), "Exited (7) 4 seconds ago");
}

#[test]
fn health_is_derived_from_status_text() {
	assert_eq!(health_from_status("Up 2 minutes (healthy)"), "healthy");
	assert_eq!(health_from_status("Up 1 minute (unhealthy)"), "unhealthy");
	assert_eq!(
		health_from_status("Up 3 seconds (health: starting)"),
		"starting"
	);
	assert_eq!(health_from_status("Exited (1) 4 seconds ago"), "");
	// A restarting container with no healthcheck must not be misread as
	// "starting" health; only the real `health: starting` token counts.
	assert_eq!(health_from_status("Restarting (1) 3 seconds ago"), "");
}

/// A running container says how long it has been up, which is what both
/// reference tools do and what podup did not. Measured the same day: `podman ps`
/// renders `Up 13 hours (healthy)` and `docker compose ps` renders
/// `Up 2 minutes`, while podup rendered the bare word `running`.
#[test]
fn a_running_container_reports_how_long_it_has_been_up() {
	let c = ContainerListEntry {
		image_id: String::new(),
		started_at: NOW - (2 * 3600 + 5 * 60 + 3),
		..entry("", "running")
	};
	assert_eq!(table_status(&c, NOW), "Up 2h 5m 3s");
}

/// The health suffix appears only when the container has a healthcheck. libpod
/// leaves `Status` empty otherwise, so there is nothing to append rather than an
/// unknown to invent.
#[test]
fn the_health_suffix_appears_only_when_there_is_a_healthcheck() {
	let started = NOW - 13 * 3600;
	let healthy = ContainerListEntry {
		image_id: String::new(),
		started_at: started,
		..entry("healthy", "running")
	};
	let unhealthy = ContainerListEntry {
		image_id: String::new(),
		started_at: started,
		..entry("unhealthy", "running")
	};
	let plain = ContainerListEntry {
		image_id: String::new(),
		started_at: started,
		..entry("", "running")
	};
	assert_eq!(table_status(&healthy, NOW), "Up 13h (healthy)");
	assert_eq!(table_status(&unhealthy, NOW), "Up 13h (unhealthy)");
	assert_eq!(table_status(&plain, NOW), "Up 13h");
}

/// A zero `StartedAt` means the field was absent, not that the container
/// started at the epoch. Rendering `Up 56y` for a container that has never run
/// is worse than saying nothing, so the cell falls back to the state.
#[test]
fn an_absent_start_time_does_not_become_an_age() {
	let c = ContainerListEntry {
		image_id: String::new(),
		started_at: 0,
		..entry("", "running")
	};
	assert_eq!(table_status(&c, NOW), "running");
}

/// A start time in the future is clock skew between this process and the
/// server, not a negative age. `Up 0s` on a container that just started is
/// right; `Up -3s` is never right.
#[test]
fn a_start_time_in_the_future_clamps_to_zero() {
	let c = ContainerListEntry {
		image_id: String::new(),
		started_at: NOW + 3,
		..entry("", "running")
	};
	assert_eq!(table_status(&c, NOW), "Up 0s");
}

/// Only a running container gets an age.
///
/// A stopped one keeps the exit code it already reported, which is the more
/// useful fact about it. The case that actually exercises the guard is
/// **paused**: it has a real `StartedAt` and is not running, so without the
/// check it would claim `Up 1h` for a container that is doing nothing. The
/// first version of this test used an exited container and proved nothing:
/// that path returns from the exit-code branch before the guard is reached, so
/// deleting the guard left it green.
#[test]
fn only_a_running_container_reports_an_age() {
	let exited = ContainerListEntry {
		image_id: String::new(),
		started_at: NOW - 3600,
		..entry_exit("exited", Some(7))
	};
	assert_eq!(table_status(&exited, NOW), "Exited (7)");

	let paused = ContainerListEntry {
		image_id: String::new(),
		started_at: NOW - 3600,
		..entry("paused", "paused")
	};
	assert_eq!(table_status(&paused, NOW), "paused");

	let created = ContainerListEntry {
		image_id: String::new(),
		started_at: NOW - 3600,
		..entry("", "created")
	};
	assert_eq!(table_status(&created, NOW), "created");
}
