use super::*;

/// #1896: libpod reads a relative `since` as a time before now, so `-30m` is
/// thirty minutes in the future and a window starting there matches nothing.
/// The validator has to reject it before any request hits libpod.
///
/// The earlier check also rejected `-1`/`-1.5` (valid pre-epoch Unix
/// timestamps) and `-0s` (a zero offset, i.e. "now"), while letting `-.5h`
/// through; Go parses that as `-30m`, so it reproduced the bug. The rule
/// is now: reject only when the part after `-` parses exactly as a Go
/// duration (one or more `<number><unit>` segments, with at least one
/// non-zero digit overall). A string like `-1e3` is not a Go duration, so
/// it is forwarded to libpod, which reads it as a negative Unix timestamp.
#[test]
fn events_since_rejects_a_negative_relative_time() {
	for bad in ["-30m", "-1h30m", "-30s", "-.5h", "-1.5h", "-10ms", "-2us"] {
		validate_events_since(Some(bad)).expect_err(&format!(
			"{bad:?} must be rejected, not silently sent to libpod"
		));
	}
	let err = validate_events_since(Some("-30m"))
		.expect_err("--since -30m must be rejected, not silently sent to libpod");
	let msg = format!("{err}");
	assert!(
		msg.contains("--since 30m"),
		"the error must suggest the value without the leading '-'; got {msg}"
	);

	for ok in [
		None,
		Some("30m"),
		Some("1h30m"),
		Some("-1"),
		Some("-1.5"),
		Some("-0s"),
		Some("-0m"),
		Some("-1e3"),
		Some("-1E3"),
		Some("-1.5e2"),
		Some("1700000000"),
		Some("2026-01-01T00:00:00Z"),
		Some("2026-01-01T00:00:00-05:00"),
	] {
		validate_events_since(ok)
			.unwrap_or_else(|e| panic!("{ok:?} should be accepted but got {e}"));
	}
}

/// #1896: the production call site has to reject a negative relative
/// `--since` before any request goes out. The previous private-validator
/// test left the call itself uncovered: removing
/// `validate_events_since(opts.since.as_deref())?` at the top of
/// `stream_events_with_options` reverted the bug without invalidating the
/// other assertions. Pointing the engine at a path that does not exist
/// proves validation runs first: a connection error would mean the request
/// was already in flight.
#[tokio::test]
async fn stream_events_rejects_a_negative_since_before_any_request() {
	let dir = tempfile::tempdir().expect("tempdir");
	let sock = dir.path().join("missing.sock");
	let engine = Engine::with_base_dir(
		Client::new(sock.to_string_lossy().into_owned()),
		"proj".into(),
		dir.path().to_path_buf(),
	);

	let opts = EventsOptions::new(Some("-30m".into()), None, vec![]);
	let err = engine
		.stream_events_with_options(false, &opts)
		.await
		.expect_err("--since -30m must be rejected before the events stream is opened");
	let msg = format!("{err}");
	assert!(
		msg.contains("--since 30m"),
		"the error must suggest the value without the leading '-'; got {msg}"
	);
}
