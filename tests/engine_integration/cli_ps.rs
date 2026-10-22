//! CLI binary integration tests for the `ps` table.
use super::*;
use std::fs;
use std::process::Command;
use tempfile::tempdir;
#[tokio::test]
async fn cli_ps_subcommand() {
	if super::podman().await.is_none() {
		return;
	}
	let dir = tempdir().unwrap();
	let compose = dir.path().join("docker-compose.yml");
	let proj = format!("t{}-clps", std::process::id());
	fs::write(
		&compose,
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.unwrap();

	Command::new(bin())
		.args([
			"-f",
			compose.to_str().unwrap(),
			"-p",
			&proj,
			"up",
			"--detach",
		])
		.output()
		.unwrap();

	let ps = Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "ps"])
		.output()
		.unwrap();
	assert!(ps.status.success());
	// The exit code was the whole assertion here, on a command whose entire job
	// is what it prints. Reading STATUS matters especially: #590 was `ps` leaving
	// that column empty for every container, which an exit-code check cannot see.
	//
	// This used to look for the bare word `running`. STATUS reports the uptime
	// now (`Up 4s`), matching what podman and docker compose print, so the check
	// moved to that shape, and it is stricter than the old one rather than
	// looser: an empty STATUS has no `Up`, so #590 would still turn this red.
	let out = String::from_utf8_lossy(&ps.stdout);
	let row = out
		.lines()
		.find(|line| line.contains(&format!("{proj}-web-1")))
		.unwrap_or_else(|| panic!("ps did not list the running container: {out:?}"));
	let cells: Vec<&str> = row.split_whitespace().collect();
	let up = cells
		.iter()
		.position(|c| *c == "Up")
		.unwrap_or_else(|| panic!("ps printed the container without its status: {row:?}"));
	assert!(
		cells
			.get(up + 1)
			.is_some_and(|span| { span.chars().next().is_some_and(|c| c.is_ascii_digit()) }),
		"the uptime after `Up` is missing or not a span: {row:?}"
	);
	// CREATED sits between the service and STATUS. It is only ever filled by
	// parsing the RFC 3339 string libpod really sends, so a blank here is the
	// parser failing against the live server, which no unit test can see,
	// because every fixture it has was written by hand. Since #1699 the cell
	// is a phrase, `3 seconds ago` or `Less than a second ago`, so it spans
	// several cells: everything between the service and `Up`. (#1837: SERVICE
	// was added between IMAGE and CREATED, so the slice starts at index 3.)
	let created = cells[3..up].join(" ");
	assert!(
		created.ends_with(" ago")
			&& (created.starts_with("Less than")
				|| created.starts_with(|c: char| c.is_ascii_digit())),
		"ps printed no CREATED age, so the timestamp did not parse: {row:?}"
	);

	Command::new(bin())
		.args(["-f", compose.to_str().unwrap(), "-p", &proj, "down"])
		.output()
		.unwrap();
}
