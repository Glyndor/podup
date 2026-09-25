//! Tracks whether the HTTP/1 connection future behind a streaming response
//! has finished by the time the response head is in hand.
//!
//! A short response the server terminates with `Connection: close` can
//! complete the driver in the same poll that delivers the response head.
//! The streaming open path polls the driver while it waits for the head
//! (it has to, because the driver is what pushes frames into the body
//! channel) and remembers the outcome here. If the driver is already
//! settled, the open path hands [`DrivenBody`] no connection (`None`):
//! re-polling a completed future violates the `Future` contract (#1900).
//!
//! The type is kept separate from [`DrivenBody`] so this state can be
//! unit-tested with a fake connection future that resolves once and
//! panics on every subsequent poll, without standing up a socket and a
//! `Connection: close` server.
//!
//! [`DrivenBody`]: super::stream_body::DrivenBody

use std::mem;
use std::task::{Context, Poll};

use super::stream_body::ConnectionFuture;

/// Whether the HTTP/1 connection future behind a streaming response has
/// already finished, and what it finished with.
///
/// Owned by [`send_streaming`](super::Client::send_streaming) while it waits
/// for the response head. Poll the driver with [`Self::poll`] from inside
/// the head-wait `poll_fn`; once the head arrives, take the still-pending
/// driver with [`Self::take_pending`] (or learn the recorded outcome with
/// [`Self::result`]).
pub enum ConnState {
	/// The driver has not yet completed. Hand this future to
	/// [`DrivenBody::new`] via [`Self::take_pending`] when the response
	/// head arrives; the body wrapper will keep driving it inline.
	///
	/// [`DrivenBody::new`]: super::stream_body::DrivenBody::new
	Pending(ConnectionFuture),
	/// The driver finished. Hand `None` to [`DrivenBody::new`] (the body
	/// wrapper must not re-poll a completed future); [`Self::result`]
	/// carries the recorded outcome so the open path can decide whether
	/// to log a driver error.
	Done(Result<(), hyper::Error>),
}

impl ConnState {
	/// Wrap a freshly-built connection future as a pending state. The
	/// streaming open path calls this once per request, immediately
	/// after [`ConnPool::open_streaming`](super::pool::ConnPool::open_streaming)
	/// hands back its driver.
	pub fn new(conn: ConnectionFuture) -> Self {
		Self::Pending(conn)
	}

	/// Poll the still-pending driver once. Once it resolves, the state
	/// transitions to [`Self::Done`] and later calls are a no-op (no
	/// second poll of the future). Called from inside the head-wait
	/// `poll_fn` alongside the sender poll.
	pub fn poll(&mut self, cx: &mut Context<'_>) {
		if let Self::Pending(fut) = self {
			if let Poll::Ready(result) = fut.as_mut().poll(cx) {
				*self = Self::Done(result);
			}
		}
	}

	/// Yield the still-pending connection future. Returns [`None`] when
	/// the driver has already finished, which is the signal to build
	/// the body wrapper without a connection. Pair with [`Self::result`]
	/// when the caller needs to distinguish a clean close from a driver
	/// error.
	///
	/// Idempotent: once the state is [`Self::Done`], every call returns
	/// [`None`].
	pub fn take_pending(&mut self) -> Option<ConnectionFuture> {
		match mem::replace(self, Self::Done(Ok(()))) {
			Self::Pending(fut) => Some(fut),
			Self::Done(_) => None,
		}
	}

	/// The recorded driver outcome, if the driver has finished. Returns
	/// [`None`] while the driver is still in flight; pair with
	/// [`Self::take_pending`] in that case.
	pub fn result(&self) -> Option<&Result<(), hyper::Error>> {
		match self {
			Self::Pending(_) => None,
			Self::Done(result) => Some(result),
		}
	}
}

#[cfg(test)]
#[path = "conn_state_tests.rs"]
mod tests;
