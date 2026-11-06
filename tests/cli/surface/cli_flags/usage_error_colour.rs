//! The `podup: error:` label of a usage error follows `--ansi`, `NO_COLOR` and
//! the sink, like the label of every other error.
//!
//! It used to be written with `eprintln!`, so the escape bytes of the error
//! style reached a pipe, a log file and a `--ansi never` run. `podup --ansi
//! never --bogus` and docker compose's `unknown flag: --bogus` should both be
//! plain text.

use super::*;

fn usage_error(args: &[&str], no_color: bool) -> String {
	let mut cmd = Command::new(bin());
	cmd.args(args)
		.env_remove("NO_COLOR")
		.env_remove("CLICOLOR_FORCE");
	if no_color {
		cmd.env("NO_COLOR", "1");
	}
	let out = cmd.output().unwrap();
	assert_eq!(out.status.code(), Some(2), "{out:?}");
	String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn a_usage_error_to_a_pipe_is_plain() {
	let text = usage_error(&["--bogus"], false);
	assert!(
		text.starts_with("podup: error: unexpected argument '--bogus'"),
		"{text:?}"
	);
	assert!(!text.contains('\u{1b}'), "{text:?}");
}

#[test]
fn ansi_never_and_no_color_keep_it_plain() {
	assert!(!usage_error(&["--ansi", "never", "--bogus"], false).contains('\u{1b}'));
	assert!(!usage_error(&["--bogus"], true).contains('\u{1b}'));
}

#[test]
fn ansi_always_still_colours_the_label() {
	let text = usage_error(&["--ansi", "always", "--bogus"], false);
	assert!(text.contains('\u{1b}'), "{text:?}");
}
