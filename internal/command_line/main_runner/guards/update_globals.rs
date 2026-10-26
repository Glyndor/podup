//! Compose-only global value-flags that `update` parses but cannot act on,
//! and the helpers that detect them.
//!
//! `update` rewrites the binary itself rather than a compose project, so the
//! four compose-only globals (`--socket` / `--profile` / `--project-directory`
//! / `--env-file`) reach it as no-ops. Accepting them silently was a
//! misleading no-op; the check below rejects them at the trust boundary.

/// Compose-only global value-flags that `update` parses (they are declared
/// `global` on [`Cli`]) but cannot act on, since self-update rewrites the binary
/// itself rather than a compose project. Each entry is `(arg-id, user-facing
/// spelling)`.
#[cfg(feature = "update")]
const UPDATE_IRRELEVANT_GLOBALS: &[(&str, &str)] = &[
	("socket", "--socket"),
	("profile", "--profile"),
	("project_directory", "--project-directory"),
	("env_file", "--env-file"),
];

/// Return the first compose-only global flag that was supplied on the command
/// line for an `update` invocation, or `None` if none were. Re-parses the
/// already-validated argv to inspect value sources; env-sourced and default
/// values are deliberately ignored, so an exported `PODMAN_SOCKET` does not
/// break `podup update`.
#[cfg(feature = "update")]
pub(crate) fn misused_update_global() -> Option<&'static str> {
	use clap::CommandFactory;
	// Parsing already succeeded once in `parse_cli`, so this re-parse cannot fail.
	let matches = crate::cli::Cli::command().try_get_matches().ok()?;
	first_misused_global(&matches)
}

/// Core of [`misused_update_global`], split out so it can be tested against
/// matches built from a fixed argv. A global flag can surface on the root
/// matches or on the `update` subcommand matches depending on its position
/// relative to the subcommand, so both are checked.
#[cfg(feature = "update")]
pub(crate) fn first_misused_global(matches: &clap::ArgMatches) -> Option<&'static str> {
	use clap::parser::ValueSource;
	let update = matches.subcommand_matches("update");
	UPDATE_IRRELEVANT_GLOBALS.iter().find_map(|(id, flag)| {
		let from_cli = |m: &clap::ArgMatches| m.value_source(id) == Some(ValueSource::CommandLine);
		(from_cli(matches) || update.is_some_and(from_cli)).then_some(*flag)
	})
}

#[cfg(all(test, feature = "update"))]
#[path = "update_globals_tests.rs"]
mod tests;
