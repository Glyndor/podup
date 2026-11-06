//! The cap on an archive that `cp` buffers before extracting it.

use bytes::Bytes;
use http_body_util::{Full, StreamBody};

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::{archive_cap, buffer_archive, MAX_CP_ARCHIVE_BYTES};
use crate::error::ComposeError;

fn body(len: usize) -> Full<Bytes> {
	Full::new(Bytes::from(vec![7u8; len]))
}

#[tokio::test]
async fn an_archive_of_exactly_the_cap_is_buffered() {
	let bytes = buffer_archive(body(4096), 4096).await.expect("at the cap");
	assert_eq!(bytes.len(), 4096);
}

#[tokio::test]
async fn an_archive_one_byte_over_the_cap_is_refused() {
	let error = buffer_archive(body(4097), 4096).await.expect_err("over");
	let text = error.to_string();
	assert!(text.contains("exceeds 4096 bytes"), "{text}");
}

/// `chunks` pieces of 1 KiB that never say how long the body is, and how many of
/// them the reader has taken so far.
fn chunked(
	chunks: usize,
) -> (
	impl hyper::body::Body<Data = Bytes, Error = std::convert::Infallible>,
	Arc<AtomicUsize>,
) {
	let taken = Arc::new(AtomicUsize::new(0));
	let counter = taken.clone();
	let piece = Bytes::from(vec![1u8; 1024]);
	let frames = (0..chunks).map(move |_| {
		counter.fetch_add(1, Ordering::SeqCst);
		Ok::<_, std::convert::Infallible>(hyper::body::Frame::data(piece.clone()))
	});
	(StreamBody::new(futures_util::stream::iter(frames)), taken)
}

/// The cap is 8 KiB and the ninth piece is the one that crosses it: the reader
/// must stop there, not read the other ninety-one before saying no.
#[tokio::test]
async fn a_chunked_archive_is_cut_off_at_the_piece_that_crosses_the_cap() {
	let (body, taken) = chunked(100);
	let error = buffer_archive(body, 8 * 1024).await.unwrap_err();
	assert!(matches!(error, ComposeError::Unsupported(_)), "{error:?}");
	assert!(error.to_string().contains("exceeds 8192 bytes"), "{error}");
	assert_eq!(taken.load(Ordering::SeqCst), 9);
}

#[tokio::test]
async fn a_chunked_archive_of_exactly_the_cap_is_buffered() {
	let (body, taken) = chunked(8);
	let bytes = buffer_archive(body, 8 * 1024).await.expect("at the cap");
	assert_eq!(bytes.len(), 8 * 1024);
	assert_eq!(taken.load(Ordering::SeqCst), 8);
}

#[test]
fn the_default_cap_stays_one_gibibyte() {
	assert_eq!(MAX_CP_ARCHIVE_BYTES, 1024 * 1024 * 1024);
}

#[test]
fn the_cap_a_copy_applies_is_one_gibibyte_unless_set() {
	let defaults = crate::engine::CpOptions::default();
	assert_eq!(archive_cap(&defaults), 1024 * 1024 * 1024);
	let new = crate::engine::CpOptions::new(None, false, false);
	assert_eq!(archive_cap(&new), 1024 * 1024 * 1024);
	let small = crate::engine::CpOptions {
		archive_cap: Some(7),
		..Default::default()
	};
	assert_eq!(archive_cap(&small), 7);
}

/// The cap reaches the real copy: a one-file archive served by a fake Podman to
/// a destination that does not exist yet (the buffered route) lands when the
/// cap is its exact length and is refused when the cap is one byte less.
#[cfg(unix)]
mod through_the_copy {
	use super::super::super::cp_progress_tests::build_tar;
	use crate::engine::{fake_podman, CpOptions, Engine};

	const CONTAINERS: &str = r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}}]"#;

	async fn copy_with_cap(cap: usize) -> (crate::error::Result<()>, std::path::PathBuf, usize) {
		let archive = build_tar(&[("a.txt", b"x")]);
		let length = archive.len();
		let fake = fake_podman::start(move |method, target| match method {
			"GET" if target.contains("/containers/json") => (200, CONTAINERS.to_string()),
			"GET" if target.contains("/archive") => (200, archive.clone()),
			_ => (404, r#"{"message":"not found"}"#.to_string()),
		});
		let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
		let file = crate::parse_str("services:\n  web:\n    image: alpine\n").unwrap();
		let dir = tempfile::tempdir().unwrap();
		let dst = dir.path().join("fresh.txt");
		let opts = CpOptions {
			archive_cap: Some(cap),
			..Default::default()
		};
		let got = engine
			.cp_with_options(&file, "web:/a.txt", dst.to_str().unwrap(), opts)
			.await;
		let kept = dir.keep();
		(got, kept.join("fresh.txt"), length)
	}

	#[tokio::test]
	async fn an_archive_of_exactly_the_cap_is_copied() {
		let length = build_tar(&[("a.txt", b"x")]).len();
		let (got, dst, _) = copy_with_cap(length).await;
		got.expect("an archive at the cap is copied");
		assert_eq!(std::fs::read(&dst).unwrap(), b"x");
		std::fs::remove_dir_all(dst.parent().unwrap()).ok();
	}

	#[tokio::test]
	async fn an_archive_one_byte_over_the_cap_is_refused_and_nothing_lands() {
		let length = build_tar(&[("a.txt", b"x")]).len();
		let (got, dst, _) = copy_with_cap(length - 1).await;
		let error = got.expect_err("one byte over the cap");
		assert!(
			error.to_string().contains("container archive exceeds"),
			"{error}"
		);
		assert!(!dst.exists(), "{} was written", dst.display());
		std::fs::remove_dir_all(dst.parent().unwrap()).ok();
	}
}
