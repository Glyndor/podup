/// 130 is 128 + SIGINT, and it is what `docker compose up` returns for
/// SIGTERM too, measured against v5.1.3 rather than derived from the signal
/// number, which would have said 143. podup returned 0 for both, so a
/// cancelled CI job reported success.
#[test]
fn an_interrupt_maps_onto_the_shell_convention() {
	assert_eq!(interrupt_exit_code(), 130);
}
use super::*;
use clap::CommandFactory;
use clap::Parser;

fn matches_for(args: &[&str]) -> clap::ArgMatches {
	Cli::command()
		.try_get_matches_from(args)
		.expect("args parse")
}

/// `#2014`: `podup events` takes an optional `[SERVICE...]` positional,
/// matching `docker compose events`. clap's plain positional lets `--format`,
/// `--since`, `--until`, and `--filter` parse in any order; this test pins
/// the parsed struct rather than just the absence of a clap usage error, so a
/// future change that drops the field or renames it shows up here.
#[test]
fn events_parses_an_optional_service_positional() {
	let cli = Cli::try_parse_from(["podup", "events", "web", "db", "--format", "json"])
		.expect("events web db --format json must parse");
	let services = match &cli.command {
		Commands::Events { services, .. } => services.clone(),
		_ => panic!("expected the events subcommand"),
	};
	assert_eq!(
		services,
		vec!["web".to_string(), "db".to_string()],
		"the service positional must carry every name in order"
	);
	let format = match &cli.command {
		Commands::Events { format, .. } => *format,
		_ => panic!("expected the events subcommand"),
	};
	assert!(
		matches!(format, EventsFormat::Json),
		"--format json must select Json"
	);

	// No positional, no service names; the whole-project feed.
	let cli =
		Cli::try_parse_from(["podup", "events"]).expect("events must parse with no positional");
	let services = match &cli.command {
		Commands::Events { services, .. } => services.clone(),
		_ => panic!("expected the events subcommand"),
	};
	assert!(
		services.is_empty(),
		"`events` with no positional must leave services empty, got {services:?}"
	);
}

#[test]
fn update_flags_compose_globals_before_subcommand_are_rejected() {
	let m = matches_for(&[
		"podup",
		"--socket",
		"unix:///tmp/x.sock",
		"update",
		"--check",
	]);
	assert_eq!(first_misused_global(&m), Some("--socket"));
}

#[test]
fn update_flags_compose_globals_after_subcommand_are_rejected() {
	let m = matches_for(&["podup", "update", "--project-directory", "/tmp"]);
	assert_eq!(first_misused_global(&m), Some("--project-directory"));
}

#[test]
fn update_without_compose_globals_is_accepted() {
	let m = matches_for(&["podup", "update", "--check", "--force"]);
	assert_eq!(first_misused_global(&m), None);
}
