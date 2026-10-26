//! Arguments of the commands that read and report: `config`, `events`, `logs`
//! and `ps`.

mod config;
mod events;
mod logs;
mod ps;

pub(crate) use config::ConfigArgs;
pub(crate) use events::EventsArgs;
pub(crate) use logs::LogsArgs;
pub(crate) use ps::PsArgs;
