//! What the build request sends to the libpod build endpoint.
//!
//! Three decisions make up the request: which Dockerfile the endpoint should
//! see and whether the body is a streamed context tar or nothing at all
//! ([`body_plan`]), the `secrets=` specs the `build.secrets` entries resolve to
//! ([`secrets`]), and the streaming transport that keeps a multi-gigabyte
//! context out of the process RSS ([`stream`]). [`service`](../service/index.html)
//! assembles them into the `POST /libpod/build?` call.

mod body_plan;
mod secrets;
mod stream;

pub(in crate::backends::engine::images::build) use body_plan::plan_build;
pub(in crate::backends::engine::images::build) use stream::{context_body, ContextSource};

#[cfg(all(test, unix))]
mod query_tests;
