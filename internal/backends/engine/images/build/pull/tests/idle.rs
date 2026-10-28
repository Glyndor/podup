use std::sync::{
	atomic::{AtomicBool, Ordering},
	Arc,
};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::StreamBody;
use hyper::body::Frame;

use super::super::idle::parse_idle_timeout;
use crate::libpod::types::image::ImagePullProgress;
use crate::libpod::types::stream::parse_json_lines_with_idle_timeout;

struct DropProbe(Arc<AtomicBool>);

impl Drop for DropProbe {
	fn drop(&mut self) {
		self.0.store(true, Ordering::SeqCst);
	}
}

#[tokio::test(start_paused = true)]
async fn stalled_pull_body_times_out_and_is_dropped() {
	let dropped = Arc::new(AtomicBool::new(false));
	let probe = DropProbe(dropped.clone());
	let frames = futures_util::stream::unfold(probe, |probe| async move {
		std::future::pending::<()>().await;
		Some((Ok::<_, hyper::Error>(Frame::data(Bytes::new())), probe))
	});
	let body = StreamBody::new(Box::pin(frames));
	let mut stream =
		parse_json_lines_with_idle_timeout::<ImagePullProgress, _>(body, Duration::from_secs(5));
	let err = tokio::time::timeout(Duration::from_secs(6), stream.next())
		.await
		.expect("the inactivity limit must finish before the outer guard")
		.expect("a stalled body is an error, not EOF")
		.err()
		.expect("the stalled body must report an error");
	assert!(err.to_string().contains("inactivity"));
	assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test(start_paused = true)]
async fn partial_json_frames_reset_inactivity_until_the_record_is_complete() {
	let parts: Vec<&'static [u8]> = vec![b"{\"stream\":", b"\"pulling", b" in fragments\"}\n"];
	let frames = futures_util::stream::unfold(parts.into_iter(), |mut parts| async move {
		let part = parts.next()?;
		tokio::time::sleep(Duration::from_secs(4)).await;
		Some((
			Ok::<_, hyper::Error>(Frame::data(Bytes::from_static(part))),
			parts,
		))
	});
	let mut stream = parse_json_lines_with_idle_timeout::<ImagePullProgress, _>(
		StreamBody::new(Box::pin(frames)),
		Duration::from_secs(5),
	);
	let started = tokio::time::Instant::now();
	assert_eq!(
		stream.next().await.unwrap().unwrap().stream,
		"pulling in fragments"
	);
	assert!(started.elapsed() > Duration::from_secs(5));
	assert!(stream.next().await.is_none());
}

#[tokio::test(start_paused = true)]
async fn inactivity_limit_restarts_after_a_progress_record() {
	let frames = futures_util::stream::unfold(false, |sent| async move {
		if sent {
			std::future::pending::<()>().await;
		}
		Some((
			Ok::<_, hyper::Error>(Frame::data(Bytes::from_static(
				b"{\"stream\":\"progress\"}\n",
			))),
			true,
		))
	});
	let mut stream = parse_json_lines_with_idle_timeout::<ImagePullProgress, _>(
		StreamBody::new(Box::pin(frames)),
		Duration::from_secs(5),
	);
	assert!(stream.next().await.unwrap().is_ok());
	assert!(tokio::time::timeout(Duration::from_secs(6), stream.next())
		.await
		.unwrap()
		.unwrap()
		.is_err());
}

#[test]
fn pull_idle_timeout_is_positive_and_configurable_without_echoing_input() {
	assert_eq!(parse_idle_timeout(None).unwrap(), Duration::from_secs(300));
	assert_eq!(
		parse_idle_timeout(Some("15")).unwrap(),
		Duration::from_secs(15)
	);
	for raw in ["0", "-1", "", "not-a-duration", "18446744073709551616"] {
		assert_eq!(
			parse_idle_timeout(Some(raw)).unwrap_err().to_string(),
			"build error: PODUP_PULL_IDLE_TIMEOUT must be a positive integer number of seconds"
		);
	}
}
