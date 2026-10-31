//! Deciding whether a file system event matters to a watch rule: the event filter, the ignore and include rules, and write-back detection.

pub(in crate::backends::engine::host_files::watch) mod events;
pub(in crate::backends::engine::host_files::watch) mod ignore_filter;
pub(in crate::backends::engine::host_files::watch) mod writeback;
