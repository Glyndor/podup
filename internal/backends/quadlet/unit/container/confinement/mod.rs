//! What the container may do: capabilities, user namespace, supplementary
//! groups, security options, and resource limits.

mod privileges;
mod resources;

pub(super) use privileges::{emit_capabilities, emit_group_add, emit_security_opts, emit_userns};
pub(super) use resources::{emit_deploy_limits, emit_limits, emit_ulimits};
