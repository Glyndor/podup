//! The ways a request leaves: buffered, streaming, and raw-hijacked.
//!
//! [`buffered`] acquires a pooled connection, issues one request and pairs
//! the response with the pool guard that has to outlive the body. [`streaming`]
//! opens a dedicated connection instead and hands the response back with the
//! body driving that connection inline; it also holds the size-capped body read
//! both paths share. [`hijack`] is the third shape: for an interactive exec the
//! request is written by hand and the socket is given back raw, because there
//! is no response to return.

use crate::connection::libpod::client::pool::PoolGuard;
use crate::connection::libpod::client::request::transport::{
	has_connection_close, BoxBody, CONNECT_TIMEOUT, MAX_RESPONSE_BYTES, READ_TIMEOUT,
};
use crate::connection::libpod::client::stream::conn_state::ConnState;
use crate::connection::libpod::client::stream::stream_body::DrivenBody;
use crate::connection::libpod::client::transport::stream::SocketStream;
use crate::connection::libpod::client::{Client, PodmanError, Result};

mod buffered;
pub(in crate::connection::libpod::client) mod hijack;
mod streaming;

#[cfg(test)]
#[path = "streaming_close_tests.rs"]
mod streaming_close_tests;
