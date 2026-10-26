//! Warnings about compose fields that weaken the isolation between a container and its host, or that rootless Podman cannot honour.

pub(in crate::backends::engine::workload::container) mod host_mode;
pub(in crate::backends::engine::workload::container) mod rootless;
