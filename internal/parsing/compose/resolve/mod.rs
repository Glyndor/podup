//! Turning a parsed compose file into what the engine runs: effective
//! `depends_on`, topological order, `pre_start` init steps, and validation.

pub(crate) mod dependencies;
pub(super) mod order;
pub mod pre_start;
pub(super) mod validate;
