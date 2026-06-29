//! CLI startup helpers: diagnostic log formatting, tracing initialization, the
//! internal-error notice, and argument parsing with framed help output.

use std::process;

use clap::Parser;
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Commands};

mod config_render;
pub(crate) use config_render::{render_config, ConfigOutput};

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

/// Validate the resolved project name at the trust boundary, before it reaches
/// any code path that builds a filesystem path from it (staging, lock files,
/// quadlet generation) or filters containers by it. Shared by the mutating
/// dispatch path and the read-only `config`/`ps` paths so every command reports
/// the same invalid-name error.
pub(crate) fn validate_project_name(project: &str) -> podup::Result<()> {
	if podup::is_safe_project_name(project) {
		Ok(())
	} else {
		Err(podup::ComposeError::Unsupported(format!(
			"project name {project:?} is not a safe path component: use only ASCII \
			 letters, digits, '-', '_', '.', not starting with '.', max 128 chars"
		)))
	}
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

/// Extract the `docker compose run -l/--label KEY=VAL` ad-hoc labels for the
/// engine builder ([`podup::Engine::with_run_labels`]). Carried on the engine
/// rather than the frozen `RunOverrides` struct so the 1.0 library API stays
/// stable, mirroring `run_overrides_for`.
pub(crate) fn run_labels_for(command: &Commands) -> Vec<String> {
	match command {
		Commands::Run { label, .. } => label.clone(),
		_ => Vec::new(),
	}
}

/// Parse the CLI, framing `--help`/`--version` output with a blank line top and
/// bottom (clap trims template edges, so wrap the rendered text here).
pub(crate) fn parse_cli() -> Cli {
	match Cli::try_parse() {
		Ok(cli) => cli,
		Err(e) => match e.kind() {
			clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
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
			clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
				// No subcommand (top level) or a required nested subcommand (e.g.
				// `generate`) was given: print the help to stderr and exit non-zero,
				// like docker compose, so a script sees the error instead of a silent
				// success. `podup help` (the explicit Help variant) still exits 0.
				eprint!("\n{}\n", e.render());
				process::exit(2);
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
			services_only: false,
			filter: vec![],
			status: vec![],
			format: OutputFormat::Table,
			services: vec![],
		}));
		assert!(is_label_only(&Commands::Events {
			format: EventsFormat::Table,
			since: None,
			until: None,
			filter: vec![],
			json: false,
		}));
		// A command that reads service definitions is not label-only.
		assert!(!is_label_only(&Commands::Top {
			format: OutputFormat::Table,
			services: vec![],
		}));
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
