//! End-to-end checks of the `cp` progress lifecycle: that the row
//! opens with `Copying`, that a byte count appears in the captured
//! verbs, and that the row finishes with `Copied X`. All under
//! redirected stderr (the cargo-test condition), so the assertions
//! target the plain sink's events, the same way `pull_board_tests`
//! pins the pull row.

use crate::engine::fake_podman;
use crate::engine::Engine;
use crate::ui::progress::capture::Capture;
use crate::ui::progress::Kind;

/// A fake that accepts every archive call (the few the cp paths
/// make) and reports the project as having one live `web-1`
/// container so the replica lookup succeeds. Returns a hand-rolled
/// tar for `GET /archive`; the extractor only needs a parseable
/// header per entry, which is enough to drive the path that reports
/// progress.
fn engine() -> (fake_podman::FakePodman, Engine) {
	let containers = r#"[{"Names":["/proj-web-1"],"Labels":{"podup.service":"web"}}]"#.to_string();
	let fake = fake_podman::start(move |method, target| match method {
		"GET" if target.contains("/containers/json") => (200, containers.clone()),
		"GET" if target.contains("/archive") => {
			// The content is a 256 KiB fill of `'x'`, large enough that the
			// cp work takes more than one 100 ms emitter tick to drain, so
			// the row verb advances while the cp is in progress and the
			// capture sees at least one `Copying <bytes>` update before the
			// closing `Copied`. The exact size is unimportant: it is the
			// behaviour (#1845: a row that reflects work done, not a clock)
			// that is being pinned, and a 256 KiB tar is the smallest payload
			// that makes that behaviour observable on a fake socket.
			(200, build_tar(&[("a.txt", &vec![b'x'; 256 * 1024])]))
		}
		"PUT" if target.contains("/archive") => (200, r#"{}"#.to_string()),
		"HEAD" if target.contains("/archive") => (200, String::new()),
		_ => (404, r#"{"message":"not found"}"#.to_string()),
	});
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	(fake, engine)
}

/// A minimal tar archive holding one regular file with the given name
/// and content. The header layout is what libpod's archive GET
/// produces; a hand-rolled tar is enough to drive the extractor.
fn build_tar(entries: &[(&str, &[u8])]) -> String {
	let mut out = Vec::new();
	for (name, content) in entries {
		let name_bytes = name.as_bytes();
		let mut header = [0u8; 512];
		header[0..name_bytes.len()].copy_from_slice(name_bytes);
		// mode: regular file, 0644 (octal, 7 digits + NUL).
		header[100..107].copy_from_slice(b"0000644");
		// uid 0, gid 0
		header[108..115].copy_from_slice(b"0000000");
		header[116..123].copy_from_slice(b"0000000");
		// size, octal, ASCII, 11 digits + NUL.
		let size_str = format!("{:011o}", content.len());
		header[124..135].copy_from_slice(size_str.as_bytes());
		// typeflag: regular file
		header[156] = b'0';
		// checksum: sum of header bytes (with the checksum field treated as spaces).
		let mut chksum: u32 = 0;
		for (i, &b) in header.iter().enumerate() {
			if (148..156).contains(&i) {
				chksum += b' ' as u32;
			} else {
				chksum += b as u32;
			}
		}
		let chksum_str = format!("{:06o}\0 ", chksum);
		header[148..156].copy_from_slice(chksum_str.as_bytes());
		out.extend_from_slice(&header);
		out.extend_from_slice(content);
		let pad = (512 - (content.len() % 512)) % 512;
		out.resize(out.len() + pad, 0);
	}
	// Two zero blocks = end of archive.
	out.resize(out.len() + 1024, 0);
	String::from_utf8_lossy(&out).into_owned()
}

/// The board is opened with one row of kind `Cp`, named after the
/// destination side, and closed on the way out including the way
/// out of a failure. Mirrors `a_failed_pull_still_closes_the_board`
/// for the cp path.
#[tokio::test]
#[cfg(unix)]
async fn a_cp_opens_one_cp_row_and_closes_it() {
	let dir = tempfile::tempdir().expect("tempdir");
	let dst = dir.path().join("out");
	std::fs::create_dir(&dst).expect("mkdir dst");
	let (_fake, engine) = engine();
	let file = crate::parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.expect("parses");

	let capture = Capture::start();
	let _ = engine.cp(&file, "web:/a.txt", dst.to_str().unwrap()).await;

	let names = capture.names();
	assert_eq!(names.len(), 1, "one row per cp call: {names:?}");
	assert_eq!(names[0], dst.to_str().unwrap());
	assert!(
		capture.rows().iter().all(|(k, _)| *k == Kind::Cp),
		"every row is a Cp row: {:?}",
		capture.rows()
	);
	assert!(
		capture.every_board_ended(),
		"the board is closed even when the destination lookup fails"
	);
}

/// The verbs the cp command records, in order. The plain sink buffers
/// the latest transitional verb and the closing verb carries the byte
/// count; the intermediate `Copying X.X` updates that the live terminal
/// would render do appear in the capture (the board records every
/// transition, regardless of which sink consumed it) but they collapse
/// to one printed line on a non-tty because the plain sink only emits
/// at `progress::end`.
///
/// The assertion is on the byte figure, not on the verb shape. The
/// closing verb already says `Copied <bytes>` and the initial `Copying`
/// has no number; the regression that drops the byte figure from the
/// emitter leaves the row as a spinner over the destination, and a
/// suite that does not pin one of the transitional verbs would be
/// checking the frame, not the measurement (#1845).
#[tokio::test]
#[cfg(unix)]
async fn a_piped_cp_records_copying_then_copied() {
	let dir = tempfile::tempdir().expect("tempdir");
	let dst = dir.path().join("out");
	std::fs::create_dir(&dst).expect("mkdir dst");
	let (_fake, engine) = engine();
	let file = crate::parse_str(
		"services:\n  web:\n    image: alpine:latest\n    command: [\"sleep\", \"infinity\"]\n",
	)
	.expect("parses");

	let capture = Capture::start();
	engine
		.cp(&file, "web:/a.txt", dst.to_str().unwrap())
		.await
		.expect("the cp succeeds");

	let verbs: Vec<String> = capture
		.verbs()
		.into_iter()
		.filter(|(k, _, _)| *k == Kind::Cp)
		.map(|(_, _, verb)| verb)
		.collect();
	assert!(
		verbs.first().map(|v| v == "Copying").unwrap_or(false),
		"the first verb is the initial `Copying`: {verbs:?}"
	);
	// The emitter rewrites the verb every 100 ms with `Copying <bytes>`,
	// using the byte counter the container->host reader and the
	// host->container PUT body both feed. A verb that starts with
	// `Copying ` (note the trailing space) is the one that proves the
	// counter reached the verb; the bare `Copying` is the initial the
	// command sets before the work starts and is what a regression that
	// drops the byte figure from `spawn_emitter` leaves behind.
	let has_byte_verb = verbs
		.iter()
		.any(|v| v.starts_with("Copying ") && v.len() > "Copying".len());
	assert!(
		has_byte_verb,
		"at least one verb carries a byte figure (e.g. `Copying 0B` or \
		 `Copying 2.00KiB`); the initial `Copying` and the closing \
		 `Copied` alone do not pin the quantity: {verbs:?}"
	);
	let last = verbs.last().expect("at least one verb");
	assert!(
		last.starts_with("Copied ") && last.len() > "Copied".len(),
		"the closing verb carries the byte count (e.g. `Copied 2.00KiB`): \
		 {last:?}"
	);
	// The shape assertions above pass on `Copied 0B`, so on their own
	// they do not pin the producer side: a reader that stopped calling
	// `counter.add(n)` and added zero instead would leave every verb
	// well-formed and every test green (measured, #1845). The figure
	// must therefore be read as a quantity. The fixture is a 256 KiB
	// payload plus tar headers, so anything the extractor actually
	// drained lands well above this floor; the floor is deliberately
	// loose because the assertion is "the counter saw the stream", not
	// "the counter matches the tar byte for byte".
	let copied = last
		.strip_prefix("Copied ")
		.expect("checked by the assertion above");
	assert!(
		copied.ends_with("KiB") || copied.ends_with("MiB"),
		"the closing figure must be the drained size, not a zero left by \
		 a producer that stopped counting; got {last:?} for a 256 KiB \
		 fixture"
	);
	let magnitude: f64 = copied
		.trim_end_matches("KiB")
		.trim_end_matches("MiB")
		.parse()
		.unwrap_or(0.0);
	let kib = if copied.ends_with("MiB") {
		magnitude * 1024.0
	} else {
		magnitude
	};
	assert!(
		kib >= 200.0,
		"a 256 KiB fixture must report at least 200KiB copied; got {last:?}"
	);
}

/// A cp against a fake that always 500s must still close the board
/// on the way out (same contract as pull: a failed row leaves the
/// cursor visible if the board is not closed). The board is opened,
/// the work fails, the row closes with `Failed`.
#[tokio::test]
#[cfg(unix)]
async fn a_failed_cp_closes_the_board_with_failed() {
	let fake = fake_podman::start(|_method, _target| (500, r#"{"message":"nope"}"#.to_string()));
	let engine = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	let file = crate::parse_str("services:\n  web:\n    image: alpine:latest\n").expect("parses");

	let capture = Capture::start();
	let result = engine.cp(&file, "web:/a.txt", "/tmp").await;
	assert!(result.is_err(), "a 500 from the daemon must surface");
	let verbs: Vec<String> = capture
		.verbs()
		.into_iter()
		.filter(|(k, _, _)| *k == Kind::Cp)
		.map(|(_, _, verb)| verb)
		.collect();
	assert!(
		verbs.iter().any(|v| v == "Failed"),
		"the closing verb is Failed on the error path: {verbs:?}"
	);
	assert!(
		capture.every_board_ended(),
		"the board is closed even when the cp fails"
	);
}
