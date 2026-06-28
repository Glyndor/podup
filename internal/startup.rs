//! CLI startup helpers: diagnostic log formatting, tracing initialization, the
//! internal-error notice, and argument parsing with framed help output.

use std::process;

use clap::Parser;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Commands, ConfigFormat};

/// Whether a command creates, destroys, or changes the state of containers and
/// so must hold the exclusive project lock.
pub(crate) fn is_mutating(command: &Commands) -> bool {
	matches!(
		command,
		Commands::Up { .. }
			| Commands::Down { .. }
			| Commands::Start { .. }
			| Commands::Stop { .. }
			| Commands::Build { .. }
			| Commands::Rm { .. }
			| Commands::Kill { .. }
			| Commands::Pause { .. }
			| Commands::Unpause { .. }
			| Commands::Run { .. }
			| Commands::Restart { .. }
			| Commands::Scale { .. }
			| Commands::Create { .. }
	)
}

/// Whether a command is scoped purely by the `podup.project` label and never
/// reads service definitions, so it can run against a project with no compose
/// file present — matching `docker compose -p NAME events`/`ps`. These commands
/// tolerate a missing compose file at startup instead of erroring `FileNotFound`.
pub(crate) fn is_label_only(command: &Commands) -> bool {
	matches!(command, Commands::Events { .. } | Commands::Ps { .. })
}

/// Canonical project URL, reused for the bug-report hint on internal errors.
const REPO_URL: &str = "https://github.com/Glyndor/podup";

/// Event formatter that renders every diagnostic as `podup: <level>: <message>`
/// on a single line, matching the prefix used by the CLI's own `eprintln!`
/// warnings and errors. This unifies the compose forward-compat diagnostics
/// (emitted via `tracing::warn!`) with the rest of podup's user-facing output.
struct PodupFormat;

impl<S, N> FormatEvent<S, N> for PodupFormat
where
	S: Subscriber + for<'a> LookupSpan<'a>,
	N: for<'a> FormatFields<'a> + 'static,
{
	fn format_event(
		&self,
		ctx: &FmtContext<'_, S, N>,
		mut writer: Writer<'_>,
		event: &Event<'_>,
	) -> std::fmt::Result {
		let level = *event.metadata().level();
		let label = podup::ui::paint(
			level_style(level),
			level_word(level),
			podup::ui::stderr_colored(),
		);
		write!(writer, "podup: {label}: ")?;
		ctx.field_format().format_fields(writer.by_ref(), event)?;
		writeln!(writer)
	}
}

/// Map a tracing level to the user-facing word used in `podup:` output.
fn level_word(level: tracing::Level) -> &'static str {
	match level {
		tracing::Level::ERROR => "error",
		tracing::Level::WARN => "warning",
		tracing::Level::INFO => "info",
		tracing::Level::DEBUG => "debug",
		tracing::Level::TRACE => "trace",
	}
}

/// The colour for a level's word: bold red (error), bold yellow (warning), green
/// (info), dim (debug/trace).
fn level_style(level: tracing::Level) -> anstyle::Style {
	use anstyle::{AnsiColor, Style};
	match level {
		tracing::Level::ERROR => Style::new().bold().fg_color(Some(AnsiColor::Red.into())),
		tracing::Level::WARN => Style::new().bold().fg_color(Some(AnsiColor::Yellow.into())),
		tracing::Level::INFO => Style::new().fg_color(Some(AnsiColor::Green.into())),
		_ => Style::new().dimmed(),
	}
}

/// Guidance printed after an internal error or panic: where to report it and a
/// reminder to scrub secrets first. Kept off ordinary, user-correctable errors.
pub(crate) fn internal_error_notice() -> String {
	format!(
		"podup: this looks like a bug; re-run with RUST_LOG=debug and report it at {REPO_URL}/issues\n\
		 podup: redact secrets (passwords, tokens, resolved env values) from any logs before sharing"
	)
}

/// Whether a panic message denotes a broken pipe (a downstream reader closed the
/// pipe early). Rust ignores SIGPIPE, so a failing `println!`/`eprintln!` panics
/// with this message; we treat it as a clean exit rather than an internal error.
/// Pure so it can be unit-tested. Matches both the textual reason and the raw OS
/// error number (EPIPE = 32 on Linux).
pub(crate) fn is_broken_pipe_panic(msg: &str) -> bool {
	let lower = msg.to_ascii_lowercase();
	lower.contains("broken pipe") || lower.contains("os error 32")
}

/// Initialize the global tracing subscriber, written to stderr in the
/// `podup: <level>: <msg>` format so stdout stays a clean pipe. `default_level`
/// is the floor used when `RUST_LOG` is unset — `warn` for most commands (so the
/// forward-compat "unknown field" notices are never silently dropped), `info`
/// for interactive long-running ones like `watch` that should surface their
/// per-action progress. `RUST_LOG` always overrides.
pub(crate) fn init_tracing(default_level: &str) {
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level)),
		)
		.with_writer(std::io::stderr)
		.event_format(PodupFormat)
		.init();
}

/// Render `config`: validate-only (`--quiet`), service-name list (`--services`),
/// or the resolved compose file in YAML/JSON with inline secret content redacted.
pub(crate) fn render_config(
	file: &podup::compose::types::ComposeFile,
	format: &ConfigFormat,
	services: bool,
	quiet: bool,
	project: &str,
) -> podup::Result<()> {
	// Reaching here means the file parsed and merged cleanly. Validate that each
	// resolved service declares an image or a build, the same rule `up` enforces
	// — and do it before the `--quiet`/`--services` short-circuits so
	// validate-only (`--quiet`) actually validates, matching `docker compose config`.
	for (name, svc) in &file.services {
		if svc.image.is_none() && svc.build.is_none() {
			return Err(podup::ComposeError::NoImageOrBuild(name.clone()));
		}
		// Reject malformed/out-of-range port mappings (e.g. `70000:80`) here too,
		// so `config` validates them rather than accepting a mapping `up` and
		// `generate quadlet` would reject.
		podup::ports::parse_ports(&svc.ports)?;
	}
	if quiet {
		return Ok(());
	}
	if services {
		for name in file.services.keys() {
			println!("{name}");
		}
		return Ok(());
	}
	let mut redacted = file.clone();
	// Surface the resolved project name in the rendered output, like
	// `docker compose config`, rather than the file's literal `name:` (or none).
	redacted.name = Some(project.to_string());
	redacted.redact_inline_content();
	let rendered = match format {
		ConfigFormat::Json => {
			let mut v = serde_json::to_value(&redacted).map_err(|e| {
				podup::ComposeError::Unsupported(format!("failed to render config as JSON: {e}"))
			})?;
			prune_json_nulls(&mut v);
			serde_json::to_string_pretty(&v).map_err(|e| {
				podup::ComposeError::Unsupported(format!("failed to render config as JSON: {e}"))
			})?
		}
		ConfigFormat::Yaml => {
			let mut v: serde_yaml::Value =
				serde_yaml::to_value(&redacted).map_err(podup::ComposeError::Parse)?;
			prune_yaml_nulls(&mut v);
			serde_yaml::to_string(&v).map_err(podup::ComposeError::Parse)?
		}
	};
	println!("{rendered}");
	Ok(())
}

/// Drop unset keys from a JSON value so `config` output omits them (like
/// `docker compose config`) instead of a wall of `field: null` and empty
/// `field: {}` sections. Recurses first so a section that becomes empty once its
/// own nulls are dropped is itself dropped.
fn prune_json_nulls(v: &mut serde_json::Value) {
	prune_json(v, false);
}

/// `preserve_nulls` keeps null leaves at the current mapping level. It is set for
/// the value under an `environment:` key so a map-form host-passthrough var
/// (`MYVAR:` → null) is not stripped from the output — it is forwarded at runtime,
/// so `config` must show it, matching docker compose (which never drops the key).
fn prune_json(v: &mut serde_json::Value, preserve_nulls: bool) {
	match v {
		serde_json::Value::Object(map) => {
			for (k, val) in map.iter_mut() {
				prune_json(val, k == "environment");
			}
			if !preserve_nulls {
				map.retain(|_, val| !is_empty_json(val));
			}
		}
		serde_json::Value::Array(arr) => {
			for val in arr.iter_mut() {
				prune_json(val, false);
			}
		}
		_ => {}
	}
}

fn is_empty_json(v: &serde_json::Value) -> bool {
	match v {
		serde_json::Value::Null => true,
		serde_json::Value::Object(m) => m.is_empty(),
		// An empty array is kept: an explicit `command: []`/`entrypoint: []`
		// overrides the image's value, so dropping it would change meaning.
		_ => false,
	}
}

/// The YAML counterpart of [`prune_json_nulls`].
fn prune_yaml_nulls(v: &mut serde_yaml::Value) {
	prune_yaml(v, false);
}

/// YAML counterpart of [`prune_json`]; `preserve_nulls` exempts an
/// `environment:` map's null (host-passthrough) values from being dropped.
fn prune_yaml(v: &mut serde_yaml::Value, preserve_nulls: bool) {
	match v {
		serde_yaml::Value::Mapping(map) => {
			for (k, val) in map.iter_mut() {
				let child_preserve = k.as_str() == Some("environment");
				prune_yaml(val, child_preserve);
			}
			if !preserve_nulls {
				let drop: Vec<serde_yaml::Value> = map
					.iter()
					.filter(|(_, val)| is_empty_yaml(val))
					.map(|(k, _)| k.clone())
					.collect();
				for k in drop {
					map.remove(&k);
				}
			}
		}
		serde_yaml::Value::Sequence(seq) => {
			for val in seq.iter_mut() {
				prune_yaml(val, false);
			}
		}
		_ => {}
	}
}

fn is_empty_yaml(v: &serde_yaml::Value) -> bool {
	match v {
		serde_yaml::Value::Null => true,
		serde_yaml::Value::Mapping(m) => m.is_empty(),
		// Keep empty sequences: an explicit `command: []`/`entrypoint: []`
		// overrides the image's value, so dropping it would change meaning.
		_ => false,
	}
}

/// Build the `run`-only flag overrides from the parsed command. These are kept
/// off the frozen public `RunOptions` API and threaded through the engine
/// builder instead (`Engine::with_run_overrides`).
pub(crate) fn run_overrides_for(command: &Commands) -> podup::RunOverrides {
	match command {
		Commands::Run {
			user,
			workdir,
			entrypoint,
			volume,
			publish,
			interactive,
			no_deps,
			..
		} => podup::RunOverrides {
			user: user.clone(),
			workdir: workdir.clone(),
			entrypoint: entrypoint.clone(),
			volumes: volume.clone(),
			publish: publish.clone(),
			interactive: *interactive,
			no_deps: *no_deps,
		},
		_ => podup::RunOverrides::default(),
	}
}

/// Parse the CLI, framing `--help`/`--version` output with a blank line top and
/// bottom (clap trims template edges, so wrap the rendered text here).
pub(crate) fn parse_cli() -> Cli {
	match Cli::try_parse() {
		Ok(cli) => cli,
		Err(e) => match e.kind() {
			clap::error::ErrorKind::DisplayHelp
			| clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
			| clap::error::ErrorKind::DisplayVersion => {
				// `--help`/`--version` are handled by clap before `--ansi` is parsed,
				// so colour the rendered text by clap's own styling only when stdout
				// is a colour sink (TTY + no NO_COLOR); piped output stays plain and
				// byte-identical to before.
				let rendered = e.render();
				if podup::ui::stdout_colored() {
					print!("\n{}\n", rendered.ansi());
				} else {
					print!("\n{rendered}\n");
				}
				process::exit(0);
			}
			_ => e.exit(),
		},
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn label_only_covers_ps_and_events() {
		use crate::cli::{EventsFormat, OutputFormat};
		// `ps` and `events` are scoped purely by the project label, so they are
		// label-only and may run without a compose file.
		assert!(is_label_only(&Commands::Ps {
			all: false,
			quiet: false,
			format: OutputFormat::Table,
		}));
		assert!(is_label_only(&Commands::Events {
			format: EventsFormat::Table,
			json: false,
		}));
		// A command that reads service definitions is not label-only.
		assert!(!is_label_only(&Commands::Top {
			format: OutputFormat::Table,
			services: vec![],
		}));
	}

	#[test]
	fn prune_json_drops_nulls_and_empty_then_collapses() {
		let mut v = serde_json::json!({
			"image": "nginx",
			"environment": null,
			"command": [],
			"labels": {},
			"deploy": { "replicas": null }
		});
		prune_json_nulls(&mut v);
		// null fields and the section emptied by its own nulls are gone, but an
		// explicit empty array (`command: []`) survives — it overrides the image.
		assert_eq!(v, serde_json::json!({ "image": "nginx", "command": [] }));
	}

	#[test]
	fn prune_yaml_drops_nulls_and_empty() {
		let mut v: serde_yaml::Value =
			serde_yaml::from_str("image: nginx\ndns: null\nnetworks: {}\n").unwrap();
		prune_yaml_nulls(&mut v);
		let out = serde_yaml::to_string(&v).unwrap();
		assert!(out.contains("image: nginx"));
		assert!(!out.contains("dns"));
		assert!(!out.contains("networks"));
	}

	#[test]
	fn level_words_match_user_facing_terms() {
		assert_eq!(level_word(tracing::Level::WARN), "warning");
		assert_eq!(level_word(tracing::Level::ERROR), "error");
		assert_eq!(level_word(tracing::Level::INFO), "info");
		assert_eq!(level_word(tracing::Level::DEBUG), "debug");
		assert_eq!(level_word(tracing::Level::TRACE), "trace");
		// Each severity gets a distinct style; debug/trace share the dim style.
		assert_ne!(
			level_style(tracing::Level::ERROR),
			level_style(tracing::Level::INFO)
		);
		assert_ne!(
			level_style(tracing::Level::WARN),
			level_style(tracing::Level::ERROR)
		);
		assert_eq!(
			level_style(tracing::Level::DEBUG),
			level_style(tracing::Level::TRACE)
		);
	}

	fn sample_file() -> podup::compose::types::ComposeFile {
		podup::parse_str("services:\n  web:\n    image: nginx\n  db:\n    image: postgres\n")
			.unwrap()
	}

	#[test]
	fn render_config_quiet_is_validate_only() {
		// `--quiet` validates and prints nothing, returning Ok.
		render_config(&sample_file(), &ConfigFormat::Yaml, false, true, "proj").unwrap();
	}

	#[test]
	fn render_config_services_lists_names() {
		// `--services` reaches the service-name listing branch without error.
		render_config(&sample_file(), &ConfigFormat::Yaml, true, false, "proj").unwrap();
	}

	#[test]
	fn render_config_yaml_and_json_render_ok() {
		render_config(&sample_file(), &ConfigFormat::Yaml, false, false, "proj").unwrap();
		render_config(&sample_file(), &ConfigFormat::Json, false, false, "proj").unwrap();
	}

	#[test]
	fn render_config_injects_resolved_project_name() {
		// The rendered output carries the resolved project name, not the file's
		// literal `name:` (here unset). Render into a buffer via the same path.
		let mut redacted = sample_file();
		redacted.name = Some("myproj".to_string());
		let v: serde_yaml::Value = serde_yaml::to_value(&redacted).unwrap();
		let out = serde_yaml::to_string(&v).unwrap();
		assert!(
			out.contains("name: myproj"),
			"config should render the resolved name"
		);
	}

	#[test]
	fn prune_preserves_environment_map_nulls() {
		// A map-form host-passthrough var (`MYVAR:` → null) survives pruning, while
		// an unrelated null elsewhere is still dropped.
		let mut v: serde_yaml::Value = serde_yaml::from_str(
			"services:\n  web:\n    image: nginx\n    dns: null\n    environment:\n      MYVAR: null\n      SET: value\n",
		)
		.unwrap();
		prune_yaml_nulls(&mut v);
		let out = serde_yaml::to_string(&v).unwrap();
		assert!(out.contains("MYVAR"), "passthrough env var must be kept");
		assert!(out.contains("SET"));
		assert!(!out.contains("dns"), "unrelated null must still be dropped");

		let mut j = serde_json::json!({
			"services": { "web": {
				"image": "nginx",
				"dns": null,
				"environment": { "MYVAR": null, "SET": "value" }
			}}
		});
		prune_json_nulls(&mut j);
		let env = &j["services"]["web"]["environment"];
		assert!(
			env.get("MYVAR").is_some(),
			"passthrough env var must be kept"
		);
		assert!(j["services"]["web"].get("dns").is_none());
	}

	#[test]
	fn broken_pipe_panic_detected() {
		assert!(is_broken_pipe_panic(
			"failed printing to stdout: Broken pipe (os error 32)"
		));
		assert!(is_broken_pipe_panic("Broken pipe"));
		assert!(!is_broken_pipe_panic("some other internal error"));
	}

	#[test]
	fn internal_error_notice_reports_and_warns_on_secrets() {
		let notice = internal_error_notice();
		assert!(notice.contains(REPO_URL), "points at the issue tracker");
		assert!(notice.contains("/issues"));
		assert!(
			notice.contains("redact"),
			"reminds the user to scrub secrets"
		);
		assert!(
			notice.contains("RUST_LOG=debug"),
			"tells the user what to capture"
		);
	}
}
