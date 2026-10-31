use super::*;

#[test]
fn label_only_covers_ps_and_events() {
	use crate::cli::{EventsFormat, OutputFormat};
	// `ps` and `events` are scoped purely by the project label, so they are
	// label-only and may run without a compose file.
	assert!(is_label_only(&Commands::Ps(PsArgs {
		all: false,
		quiet: false,
		services_only: false,
		size: false,
		filter: vec![],
		status: vec![],
		format: OutputFormat::Table,
		services: vec![],
	})));
	assert!(is_label_only(&Commands::Events(EventsArgs {
		format: EventsFormat::Table,
		since: None,
		until: None,
		filter: vec![],
		json: false,
		services: vec![],
	})));
	// A command that reads service definitions is not label-only.
	assert!(!is_label_only(&Commands::Top {
		format: OutputFormat::Table,
		services: vec![],
	}));
}

/// With `#[non_exhaustive]` on `Commands`, every variant of the enum
/// must be reachable through `is_label_only` without the test going stale.
/// The match is exhaustive in this crate (the `#[non_exhaustive]` only
/// bites across the crate boundary), so adding a new variant forces this
/// test to fail to compile until the new command is classified.
#[test]
fn is_label_only_classifies_every_variant() {
	use crate::cli::{
		AutostartCommands, ConfigFormat, EventsFormat, GenerateCommands, OutputFormat,
	};
	// `Commands` is not `Debug` (clap's derive doesn't add it); assert
	// by index instead of formatting the value.
	let cases: Vec<(&'static str, Commands, bool)> = vec![
		(
			"Up",
			Commands::Up(UpArgs {
				detach: false,
				build: false,
				watch: false,
				remove_orphans: false,
				no_recreate: false,
				force_recreate: false,
				no_deps: false,
				timeout: None,
				scale: vec![],
				pull: None,
				no_build: false,
				quiet_pull: false,
				wait: false,
				wait_timeout: None,
				no_start: false,
				timestamps: false,
				renew_anon_volumes: false,
				abort_on_container_exit: false,
				exit_code_from: None,
				services: vec![],
			}),
			false,
		),
		(
			"Ps",
			Commands::Ps(PsArgs {
				all: false,
				quiet: false,
				services_only: false,
				size: false,
				filter: vec![],
				status: vec![],
				format: OutputFormat::Table,
				services: vec![],
			}),
			true,
		),
		(
			"Events",
			Commands::Events(EventsArgs {
				format: EventsFormat::Table,
				since: None,
				until: None,
				filter: vec![],
				json: false,
				services: vec![],
			}),
			true,
		),
		(
			"Config",
			Commands::Config(ConfigArgs {
				format: ConfigFormat::Yaml,
				services: false,
				volumes: false,
				images: false,
				profiles: false,
				hash: None,
				quiet: false,
				no_interpolate: false,
				no_normalize: false,
				resolve_image_digests: false,
			}),
			false,
		),
		(
			"Generate",
			Commands::Generate {
				kind: GenerateCommands::Quadlet { output: None },
			},
			false,
		),
		(
			"Autostart",
			Commands::Autostart {
				kind: AutostartCommands::Status,
			},
			false,
		),
	];
	for (name, cmd, expected) in cases {
		assert_eq!(
			is_label_only(&cmd),
			expected,
			"wrong classification for {name}"
		);
	}
}
