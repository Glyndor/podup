//! How a lifecycle step runs once it is chosen: walking services in parallel, the ordered pre-start containers, and re-checking an operation whose response was dropped.

pub(in crate::backends::engine::lifecycle) mod drop_recheck;
pub(in crate::backends::engine) mod parallel;
pub(in crate::backends::engine::lifecycle) mod pre_start;
