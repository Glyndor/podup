//! Building what goes out on the wire: the request itself, the percent-encoding
//! of a path segment, and the transport-level pieces every request shares (the
//! boxed body, the per-request ceilings, the generic
//! [`apply_timeout`](crate::libpod::client::Client::apply_timeout) wrapper).
//!
//! [`build`] owns the request shape (origin-form URI, `Host: localhost`,
//! optional content type), [`encode`] turns an arbitrary string into one safe
//! path segment, and [`transport`] holds the items that are shared by every
//! request path so a new one does not have to re-derive them.

use super::{BoxBody, Client, PodmanError, Result};

mod build;
pub(in crate::connection::libpod::client) mod encode;
pub(in crate::connection::libpod::client) mod transport;

#[cfg(test)]
#[path = "build_tests.rs"]
mod build_tests;

#[cfg(test)]
#[path = "transport_tests.rs"]
mod transport_tests;
