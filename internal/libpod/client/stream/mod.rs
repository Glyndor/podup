//! The streaming response body and the state of the connection behind it.
//!
//! [`stream_body`] is the body that polls its own HTTP/1 connection in line
//! with each frame, so no cross-task wake-up sits between a decoded frame and
//! the read that consumes it. [`conn_state`] records whether that connection
//! finished before the response head was in hand, which is the case a short
//! `Connection: close` response produces and the `Future` contract forbids
//! re-polling (#1900). The two are one concern: the body is what makes the
//! state worth tracking.

pub(in crate::libpod::client) mod conn_state;
pub(in crate::libpod::client) mod stream_body;
