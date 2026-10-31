//! One HTTP exchange with Podman: building the request, the ways it is sent, and reading the response.

pub(in crate::connection::libpod::client) mod request;
pub(in crate::connection::libpod::client) mod response;
pub(in crate::connection::libpod::client) mod send;
