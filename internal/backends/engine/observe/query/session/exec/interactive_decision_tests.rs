use super::{wants_interactive, wants_stdin, ExecOptions};

/// #1079: `-T` opts out, matching `docker compose exec`, which has no `-i`
/// because a TTY on both ends is the default.
#[test]
fn no_tty_flag_disables_the_pty() {
	let opts = ExecOptions::default().with_no_tty_for_test(true);
	assert!(!wants_interactive(&opts, true));
}

/// `-d` detaches, so there is nobody to be interactive with.
#[test]
fn detach_disables_the_pty() {
	let opts = ExecOptions::default().with_detach_for_test(true);
	assert!(!wants_interactive(&opts, true));
}

/// The decisive one for existing users: with stdin not a terminal (any
/// script or pipeline) `exec` stays on the unchanged streaming path.
/// Allocating a pty there would change output framing for every script that
/// already calls `podup exec`.
#[test]
fn a_non_terminal_stdin_stays_on_the_streaming_path() {
	assert!(!wants_interactive(&ExecOptions::default(), false));
}

/// And the positive case, which the old ambient-stdin test could never
/// assert: defaults plus a terminal is what turns the pty on.
#[test]
fn a_terminal_stdin_with_defaults_is_interactive() {
	assert!(wants_interactive(&ExecOptions::default(), true));
}

/// Standard input is a decision of its own: a pipe or a redirected file is
/// exactly the case `exec -T db psql < dump.sql` is made of, and the session
/// has to carry it even though no pseudo-terminal is allocated.
#[test]
fn a_non_terminal_stdin_is_attached_to_the_session() {
	assert!(wants_stdin(&ExecOptions::default(), false));
}

/// `-T` opts out of the pseudo-terminal, not of the input: the file the
/// operator redirected is still the file the command has to read.
#[test]
fn no_tty_still_attaches_a_non_terminal_stdin() {
	let opts = ExecOptions::default().with_no_tty_for_test(true);
	assert!(wants_stdin(&opts, false));
}

/// A detached session has no reader to carry the input, and nobody to
/// deliver it to.
#[test]
fn detach_does_not_attach_the_stdin() {
	let opts = ExecOptions::default().with_detach_for_test(true);
	assert!(!wants_stdin(&opts, false));
}

/// A terminal on stdin is not attached on the non-interactive path: the
/// command would wait for input that a redirected run never provides, and a
/// session that used to answer at once would hang instead.
#[test]
fn a_terminal_stdin_is_not_attached_without_a_pty() {
	assert!(!wants_stdin(&ExecOptions::default(), true));
}
