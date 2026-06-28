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
	match v {
		serde_json::Value::Object(map) => {
			for val in map.values_mut() {
				prune_json_nulls(val);
			}
			map.retain(|_, val| !is_empty_json(val));
		}
		serde_json::Value::Array(arr) => {
			for val in arr.iter_mut() {
				prune_json_nulls(val);
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
	match v {
		serde_yaml::Value::Mapping(map) => {
			for (_, val) in map.iter_mut() {
				prune_yaml_nulls(val);
			}
			let drop: Vec<serde_yaml::Value> = map
				.iter()
				.filter(|(_, val)| is_empty_yaml(val))
				.map(|(k, _)| k.clone())
				.collect();
			for k in drop {
				map.remove(&k);
			}
		}
		serde_yaml::Value::Sequence(seq) => {
			for val in seq.iter_mut() {
				prune_yaml_nulls(val);
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
		render_config(&sample_file(), &ConfigFormat::Yaml, false, true).unwrap();
	}

	#[test]
	fn render_config_services_lists_names() {
		// `--services` reaches the service-name listing branch without error.
		render_config(&sample_file(), &ConfigFormat::Yaml, true, false).unwrap();
	}

	#[test]
	fn render_config_yaml_and_json_render_ok() {
		render_config(&sample_file(), &ConfigFormat::Yaml, false, false).unwrap();
		render_config(&sample_file(), &ConfigFormat::Json, false, false).unwrap();
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
