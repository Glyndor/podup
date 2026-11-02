//! One-off `run` command: start a throwaway container for a service, stream its
//! output, and remove it when done.

mod attached;
mod command;
mod completion;
mod container_lookup;
mod deps_wait;
mod environment;
mod init_service;
pub(super) mod options;
mod overrides;

use completion::finish_run;
use environment::merge_run_environment;
