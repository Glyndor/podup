use super::*;

/// The user-facing error for a refused entry names the entry's path. The
/// previous bool return forced the caller to invent a message that named
/// neither the entry nor the stat; this pins that the message now names it.
#[test]
fn the_refusal_names_the_entry_path() {
	let failure = LandedFailure::Mismatch {
		path: "payload/link".to_string(),
		expected: SentKind::Link("anywhere".to_string()),
		stat: Some(stat(0, LINK_MODE)),
	};
	let msg = format_landed_failure(&failure, "/tmp");
	assert!(
		msg.contains("payload/link"),
		"the message must name the entry that failed: {msg}"
	);
	assert!(
		msg.contains("/tmp"),
		"the message must name the directory the archive was extracted at: {msg}"
	);
}

/// The user-facing error carries the literal PathStat the runtime answered,
/// not just a verdict. The intent is that the next attempt's log shows the
/// stat that was read for the failing entry, so a diagnosis can compare
/// expected and answered without re-running the upload.
#[test]
fn the_refusal_carries_the_stat_that_was_read() {
	let failure = LandedFailure::Mismatch {
		path: "payload/link".to_string(),
		expected: SentKind::Link("anywhere".to_string()),
		stat: Some(stat(0, LINK_MODE)),
	};
	let msg = format_landed_failure(&failure, "/tmp");
	// The Debug rendering of `PathStat` is what carries the stat; the field
	// names (size, mode) are part of the struct and end up in the message.
	assert!(
		msg.contains("size"),
		"the message must include the literal PathStat, whose Debug includes \
		 the field name `size`: {msg}"
	);
	assert!(
		msg.contains("mode"),
		"the message must include the literal PathStat, whose Debug includes \
		 the field name `mode`: {msg}"
	);
	// The expected kind is part of the message too: a future diagnosis needs
	// to know what the upload was supposed to land.
	assert!(
		msg.contains("Link"),
		"the message must name the entry kind that was expected: {msg}"
	);
}

/// A 404-without-stat shape (only reachable for symlinks on runtimes that
/// drop the header) reports the missing stat explicitly. `None` on its own
/// would be ambiguous to a reader who has not seen the dispatch.
#[test]
fn the_refusal_for_a_404_without_stat_explains_the_missing_stat() {
	let failure = LandedFailure::Mismatch {
		path: "payload/link".to_string(),
		expected: SentKind::Link("anywhere".to_string()),
		stat: None,
	};
	let msg = format_landed_failure(&failure, "/tmp");
	assert!(
		msg.contains("payload/link"),
		"the path is named even when the stat is missing: {msg}"
	);
	assert!(
		!msg.contains("None"),
		"`None` alone is ambiguous; the message must say no stat was returned: {msg}"
	);
	assert!(
		msg.contains("no stat"),
		"the message must explain the missing stat: {msg}"
	);
}

/// A stat request that errored out (a 5xx, a transport drop) reports the
/// runtime's message, so the next diagnosis can see why the verification
/// could not be done.
#[test]
fn the_refusal_for_a_stat_error_carries_the_runtime_message() {
	let failure = LandedFailure::StatError {
		path: "payload/link".to_string(),
		error: "podman transport: connection reset".to_string(),
	};
	let msg = format_landed_failure(&failure, "/tmp");
	assert!(msg.contains("payload/link"), "path is named: {msg}");
	assert!(
		msg.contains("connection reset"),
		"the runtime's message is carried verbatim: {msg}"
	);
}

/// An unreadable archive (the archive sent could not be parsed) reports the
/// reason only; no entry is named because no entry was reached.
#[test]
fn the_refusal_for_an_unreadable_archive_carries_the_reason() {
	let failure = LandedFailure::Unnamed("invalid gzip header".to_string());
	let msg = format_landed_failure(&failure, "/tmp");
	assert!(
		msg.contains("invalid gzip header"),
		"the reason is carried: {msg}"
	);
	assert!(
		!msg.contains("None"),
		"no stat is named because no stat was asked for: {msg}"
	);
}
