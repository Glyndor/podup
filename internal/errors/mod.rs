//! The crate's error type and the hint that points a rejected name at the
//! service it was meant for.

pub(crate) mod clip;
pub(crate) mod error;
pub(crate) mod redact;
/// CLI diagnostics for container names passed where a service name is required.
#[doc(hidden)]
pub mod service_hint;
