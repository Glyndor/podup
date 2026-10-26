//! Interactive terminal plumbing: the `--*-i` flag detection and the
//! stream pump that bridges a libpod `Hijacked` stream with the local
//! stdin/stdout/stderr of an attached `run`.

pub(in crate::backends::engine) mod interactive;
pub(in crate::backends::engine) mod pump;

pub(crate) use interactive::wants_interactive_run;
