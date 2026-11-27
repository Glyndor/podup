//! How the container process runs: its command, environment, user, stop and
//! restart behaviour, and logging.

mod command;
mod env;
mod env_file_check;
mod env_values;
mod lifecycle;
mod logging;
mod runtime;

pub(super) use command::emit_command;
pub(super) use env::{emit_env_files, emit_environment};
pub(super) use env_values::emit_environment_file;
pub(crate) use env_values::environment_file;
#[cfg(test)]
pub(crate) use env_values::fits_env_file;
pub(super) use lifecycle::{emit_restart, emit_stop, emit_timeouts};
pub(super) use logging::emit_logging;
pub(super) use runtime::emit_runtime;
