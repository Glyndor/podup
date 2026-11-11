use super::{ansi_from_argv, AnsiMode};
use clap::Parser;

fn argv(args: &[&str]) -> impl Iterator<Item = String> + use<> {
	args.iter()
		.map(|s| s.to_string())
		.collect::<Vec<_>>()
		.into_iter()
}

/// clap renders help before the parsed `--ansi` is applied, so the flag has
/// to be read straight off argv or `podup --ansi never --help` comes out
/// coloured, which it did, while `NO_COLOR=1 podup --help` did not.
#[test]
fn ansi_is_found_before_the_subcommand() {
	assert_eq!(
		ansi_from_argv(argv(&["podup", "--ansi", "never", "--help"])),
		Some(AnsiMode::Never)
	);
	assert_eq!(
		ansi_from_argv(argv(&["podup", "--ansi=always", "up"])),
		Some(AnsiMode::Always)
	);
}

/// It is a global flag, so it is equally valid after the subcommand.
#[test]
fn ansi_is_found_after_the_subcommand() {
	assert_eq!(
		ansi_from_argv(argv(&["podup", "up", "--ansi", "never"])),
		Some(AnsiMode::Never)
	);
}

/// No flag means no opinion; the existing auto-detection decides.
#[test]
fn absent_ansi_yields_none() {
	assert_eq!(ansi_from_argv(argv(&["podup", "up", "-d"])), None);
	assert_eq!(ansi_from_argv(argv(&["podup", "--ansi"])), None);
	assert_eq!(ansi_from_argv(argv(&["podup", "--ansi", "sideways"])), None);
}

/// `--` ends podup's options. Everything after it is the container's command,
/// which `run`/`exec` forward verbatim with `allow_hyphen_values`, so a
/// passthrough argument reading `--ansi always` is not podup's flag.
#[test]
fn ansi_after_a_double_dash_is_not_ours() {
	assert_eq!(
		ansi_from_argv(argv(&["podup", "exec", "svc", "--", "--ansi", "always"])),
		None
	);
	assert_eq!(
		ansi_from_argv(argv(&[
			"podup",
			"exec",
			"svc",
			"--",
			"sh",
			"-c",
			"--ansi=never"
		])),
		None
	);
}

/// A flag before the `--` is still ours.
#[test]
fn ansi_before_a_double_dash_is_ours() {
	assert_eq!(
		ansi_from_argv(argv(&[
			"podup", "--ansi", "never", "exec", "svc", "--", "true"
		])),
		Some(AnsiMode::Never)
	);
}

#[test]
fn connection_pool_size_rejects_excessive_values_at_parse_time() {
	for bad in ["1025".to_string(), usize::MAX.to_string()] {
		let result =
			super::Cli::try_parse_from(["podup", "--connection-pool-size", &bad, "version"]);
		assert!(result.is_err(), "pool size {bad} must be rejected");
	}
}

#[test]
fn connection_pool_size_accepts_zero_and_the_upper_boundary() {
	for value in ["0", "1", "1024"] {
		let cli = super::Cli::try_parse_from(["podup", "--connection-pool-size", value, "version"])
			.unwrap();
		assert_eq!(cli.connection_pool_size, Some(value.parse().unwrap()));
	}
}

/// An option written after the name of a service used to be swallowed as one
/// more service name, so `start web --wait` waited for nothing and
/// `stop web --timeout 0` stopped a service called `--timeout`.
#[test]
fn an_option_after_a_service_name_is_an_option() {
	use super::{Cli, Commands};
	let parse = |args: &[&str]| {
		let mut full = vec!["podup"];
		full.extend_from_slice(args);
		Cli::try_parse_from(full)
			.expect("the command line must parse")
			.command
	};
	match parse(&["start", "web", "--wait"]) {
		Commands::Start { wait, services, .. } => {
			assert!(wait, "--wait was taken as a service");
			assert_eq!(services, ["web"]);
		}
		_ => panic!("expected start"),
	}
	match parse(&["stop", "web", "--timeout", "0"]) {
		Commands::Stop { timeout, services } => {
			assert_eq!(timeout, Some(0));
			assert_eq!(services, ["web"]);
		}
		_ => panic!("expected stop"),
	}
	match parse(&["up", "web", "-d"]) {
		Commands::Up(args) => {
			assert!(args.detach, "-d was taken as a service");
			assert_eq!(args.services, ["web"]);
		}
		_ => panic!("expected up"),
	}
}

fn color_for(args: &[&str]) -> anstream::ColorChoice {
	super::color_choice(&super::Cli::try_parse_from(args).unwrap())
}

#[test]
fn up_no_color_turns_colour_off_whatever_ansi_says() {
	assert_eq!(
		color_for(&["podup", "up", "--no-color"]),
		anstream::ColorChoice::Never
	);
	assert_eq!(
		color_for(&["podup", "--ansi", "always", "up", "--no-color"]),
		anstream::ColorChoice::Never
	);
}

#[test]
fn without_no_color_the_ansi_choice_stands() {
	assert_eq!(
		color_for(&["podup", "--ansi", "always", "up"]),
		anstream::ColorChoice::Always
	);
	assert_eq!(
		color_for(&["podup", "--ansi", "always", "logs", "--no-color"]),
		anstream::ColorChoice::Always
	);
}

fn parses(args: &[&str]) -> bool {
	super::Cli::try_parse_from(args).is_ok()
}

#[test]
fn up_accepts_the_attach_and_output_options() {
	assert!(parses(&[
		"podup",
		"up",
		"--no-color",
		"--no-log-prefix",
		"--attach",
		"web",
		"--attach",
		"db",
		"--no-attach",
		"cache",
		"--quiet-build",
		"--always-recreate-deps",
		"-y",
		"--abort-on-container-failure",
	]));
	assert!(parses(&["podup", "up", "--yes", "--attach-dependencies"]));
}

#[test]
fn up_option_combinations_docker_compose_refuses_are_refused() {
	assert!(!parses(&[
		"podup",
		"up",
		"--attach",
		"web",
		"--attach-dependencies"
	]));
	assert!(!parses(&[
		"podup",
		"up",
		"--always-recreate-deps",
		"--no-recreate"
	]));
	for other in ["-d", "--wait", "--watch"] {
		assert!(!parses(&[
			"podup",
			"up",
			"--abort-on-container-failure",
			other
		]));
	}
}

/// A single-valued global option given twice keeps the last value, as docker
/// compose does, instead of failing with "cannot be used multiple times", so a
/// wrapper that always passes `-p` can be given another `-p` after it.
#[test]
fn a_repeated_project_name_keeps_the_last_one() {
	let cli = super::Cli::try_parse_from(["podup", "-p", "a", "-p", "b", "ps"])
		.expect("a repeated -p must parse");
	assert_eq!(cli.project.as_deref(), Some("b"));
}

#[test]
fn a_repeated_project_directory_keeps_the_last_one() {
	let cli = super::Cli::try_parse_from([
		"podup",
		"--project-directory",
		"/first",
		"--project-directory",
		"/second",
		"ps",
	])
	.expect("a repeated --project-directory must parse");
	assert_eq!(
		cli.project_directory,
		Some(std::path::PathBuf::from("/second"))
	);
}

#[test]
fn a_repeated_ansi_keeps_the_last_one_before_and_after_the_subcommand() {
	let before = super::Cli::try_parse_from(["podup", "--ansi", "never", "--ansi", "always", "ps"])
		.expect("a repeated --ansi must parse");
	assert_eq!(before.ansi, AnsiMode::Always);
	let after = super::Cli::try_parse_from(["podup", "ps", "--ansi", "never", "--ansi", "always"])
		.expect("a repeated --ansi after the subcommand must parse");
	assert_eq!(after.ansi, AnsiMode::Always);
}
