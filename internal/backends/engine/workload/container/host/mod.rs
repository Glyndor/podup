//! What the engine learns from the daemon about the host it runs on: whether
//! it is rootless, the log driver it defaults to, and the cgroup layout that
//! decides which limits a container can carry. One `GET /info` answers all
//! three, read once per invocation.

pub(in crate::backends::engine) mod info;
mod log_driver;
mod rootless;
