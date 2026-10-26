//! How the container process runs: its command, environment, user, stop and
//! restart behaviour, and logging.

mod command;
mod env;
mod lifecycle;
mod logging;
mod runtime;

pub(super) use command::emit_command;
pub(super) use env::{emit_env_files, emit_environment};
pub(super) use lifecycle::{emit_restart, emit_stop};
pub(super) use logging::emit_logging;
pub(super) use runtime::emit_runtime;
