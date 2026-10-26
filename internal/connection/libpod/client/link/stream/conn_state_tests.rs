//! Unit tests for [`ConnState`].
//!
//! These pin the `Future` contract [`ConnState`] exists to enforce:
//! once a connection future resolves, it must not be polled again. The
//! fake connection futures below panic on a second poll; the tests
//! exercise the exact path [`super::Client::send_streaming`] walks when
//! the head arrives after the driver finishes, and verify the
//! [`DrivenBody`](super::stream_body::DrivenBody) wrapper built from
//! the state never touches the fake future again (#1900).

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;

use super::super::stream_body::{ConnectionFuture, DrivenBody};
use super::ConnState;

/// A connection future that resolves `Ok(())` on its first poll and
/// panics on every subsequent poll. Stands in for a hyper driver that
/// has already seen the response terminator and exited cleanly; a
/// second poll violates the `Future` contract the way a buggy
/// `DrivenBody` wrapper would trigger if [`ConnState`] did not record
/// the completion.
struct OneShotReady {
	polls: Arc<AtomicUsize>,
}

impl std::future::Future for OneShotReady {
	type Output = Result<(), hyper::Error>;

	fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
		let polls = self.polls.fetch_add(1, Ordering::SeqCst);
		assert_eq!(
			polls, 0,
			"OneShotReady must not be polled again after the first poll"
		);
		Poll::Ready(Ok(()))
	}
}

fn one_shot_ready() -> (ConnectionFuture, Arc<AtomicUsize>) {
	let polls = Arc::new(AtomicUsize::new(0));
	let polls_for_fut = polls.clone();
	let fut: ConnectionFuture = Box::pin(OneShotReady {
		polls: polls_for_fut,
	});
	(fut, polls)
}

/// A connection future that stays pending forever and counts its polls.
/// Stands in for a driver that has decoded the response head but has
/// not seen the response terminator yet (the steady state for any
/// streaming body that is not `Connection: close`).
struct CountingPending {
	polls: Arc<AtomicUsize>,
}

impl std::future::Future for CountingPending {
	type Output = Result<(), hyper::Error>;

	fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
		self.polls.fetch_add(1, Ordering::SeqCst);
		Poll::Pending
	}
}

fn counting_pending() -> (ConnectionFuture, Arc<AtomicUsize>) {
	let polls = Arc::new(AtomicUsize::new(0));
	let polls_for_fut = polls.clone();
	let fut: ConnectionFuture = Box::pin(CountingPending {
		polls: polls_for_fut,
	});
	(fut, polls)
}

type OneFrameBody =
	StreamBody<futures_util::stream::Iter<std::vec::IntoIter<Result<Frame<Bytes>, hyper::Error>>>>;

/// A synthetic body that yields one data frame and then ends. The
/// `DrivenBody` wrapper polls the inner body on every `poll_frame`,
/// so this is enough to drive the body through to completion while
/// asserting that the connection future alongside it is or is not
/// being polled.
fn one_frame_body(payload: &'static [u8]) -> OneFrameBody {
	let frames = vec![Ok::<_, hyper::Error>(Frame::data(Bytes::from_static(
		payload,
	)))];
	StreamBody::new(futures_util::stream::iter(frames))
}

fn noop_cx() -> Context<'static> {
	Context::from_waker(futures_util::task::noop_waker_ref())
}

/// After the head arrives and the driver has finished in the same
/// poll, the state yields no driver. The `OneShotReady` future is
/// already settled, so a `take_pending` that returned `Some` would
/// mean `send_streaming` would hand the completed future to
/// `DrivenBody`, and the next `poll_frame` would panic on the second
/// poll. This test asserts the safe outcome directly: `take_pending`
/// returns `None`, the recorded result is the clean close, and a
/// repeated poll is a no-op (#1900).
#[tokio::test]
async fn conn_state_yields_no_driver_after_ready() {
	let (fut, polls) = one_shot_ready();
	let mut state = ConnState::new(fut);
	let mut cx = noop_cx();

	// Simulate the head-wait poll_fn: drive the connection once.
	state.poll(&mut cx);
	assert_eq!(polls.load(Ordering::SeqCst), 1, "first poll ran");

	// The driver has finished; `take_pending` yields nothing and the
	// recorded outcome is the clean close.
	assert!(
		state.take_pending().is_none(),
		"no driver to hand to DrivenBody once the state is Done"
	);
	assert!(
		matches!(state.result(), Some(Ok(()))),
		"the recorded outcome is the clean close"
	);

	// Repeated polls on the state itself are a no-op: the state does
	// not poll the future again, and the future's own panic does not
	// fire.
	state.poll(&mut cx);
	assert_eq!(
		polls.load(Ordering::SeqCst),
		1,
		"the future was not polled a second time"
	);
	assert!(
		state.take_pending().is_none(),
		"take_pending stays None once the state has finished"
	);
}

/// After the head arrives with the driver still in flight, the state
/// yields the driver so the body wrapper can keep driving it
/// inline (#1900).
#[tokio::test]
async fn conn_state_yields_pending_driver() {
	let (fut, polls) = counting_pending();
	let mut state = ConnState::new(fut);
	let mut cx = noop_cx();

	state.poll(&mut cx);
	assert_eq!(polls.load(Ordering::SeqCst), 1, "first poll ran");
	assert!(
		state.result().is_none(),
		"no result yet while the driver is still in flight"
	);

	let taken = state.take_pending();
	assert!(taken.is_some(), "driver is still in flight");

	// Dropping the placeholder the `take_pending` left behind is a
	// no-op.
	drop(state);
}

/// A `DrivenBody` built from a state whose driver has already finished
/// must never poll that driver again. The body wrapper holds no
/// connection (`None`); the fake future is dropped with the state and
/// is never observed by the body.
///
/// If [`ConnState`] ever stopped recording the completion, this test
/// would still build the `DrivenBody` with `Some(fake)`, and the body
/// read below would poll the fake on its very first `poll_frame` ---
/// the second poll overall --- panicking on the `OneShotReady`
/// assertion (#1900).
#[tokio::test]
async fn driven_body_from_finished_state_never_re_polls_the_driver() {
	let (fut, polls) = one_shot_ready();
	let mut state = ConnState::new(fut);
	let mut cx = noop_cx();
	state.poll(&mut cx);
	assert_eq!(polls.load(Ordering::SeqCst), 1, "first poll ran");

	// Whatever the state yields is exactly what production code
	// passes to `DrivenBody::new`.
	let conn = state.take_pending();
	let body = one_frame_body(b"the quick brown fox");
	let driven: DrivenBody<_> = DrivenBody::new(body, conn);

	let collected = driven
		.collect()
		.await
		.expect("body drains to completion")
		.to_bytes();
	assert_eq!(collected.as_ref(), b"the quick brown fox");

	assert_eq!(
		polls.load(Ordering::SeqCst),
		1,
		"the body wrapper never re-polled the driver"
	);
}
