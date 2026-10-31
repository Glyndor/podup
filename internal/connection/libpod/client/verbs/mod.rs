//! The HTTP verbs the client exposes.
//!
//! `ping` and the `HEAD` path-stat calls live here too: they were one
//! `misc.rs` until it split in two, and both belong to the same verb table as
//! `get`, `post`, `put` and `delete`. None of them owns connection handling:
//! every method builds its request with
//! [`Client::build_request`](super::Client::build_request) and hands it to the
//! buffered path ([`Client::send`](super::Client::send)) or to the streaming
//! path ([`Client::send_streaming`](super::Client::send_streaming)).

use super::{full, meets_minimum, Client, DrivenBody, PathStat, PodmanError, Result, READ_TIMEOUT};

mod delete;
mod get;
mod head;
mod ping;
mod post;
mod put;
