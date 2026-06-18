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
		write!(writer, "podup: {}: ", level_word(*event.metadata().level()))?;
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

/// Guidance printed after an internal error or panic: where to report it and a
/// reminder to scrub secrets first. Kept off ordinary, user-correctable errors.
pub(crate) fn internal_error_notice() -> String {
	format!(
		"podup: this looks like a bug; re-run with RUST_LOG=debug and report it at {REPO_URL}/issues\n\
		 podup: redact secrets (passwords, tokens, resolved env values) from any logs before sharing"
	)
}

/// Initialize the global tracing subscriber: warnings by default (so the
/// forward-compat "unknown field" notices are never silently dropped), written
/// to stderr in the `podup: <level>: <msg>` format so stdout stays a clean pipe.
pub(crate) fn init_tracing() {
	tracing_subscriber::fmt()
		.with_env_filter(
			EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
		)
		.with_writer(std::io::stderr)
		.event_format(PodupFormat)
		.init();
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
				print!("\n{e}\n");
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
	fn level_words_match_user_facing_terms() {
		assert_eq!(level_word(tracing::Level::WARN), "warning");
		assert_eq!(level_word(tracing::Level::ERROR), "error");
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
