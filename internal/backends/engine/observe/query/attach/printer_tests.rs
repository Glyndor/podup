use super::ChunkPrinter;

/// Feeds `payload` to a printer in two frames cut at `cut` and returns what
/// would have been printed on stdout, in order.
fn printed(payload: &[u8], cut: usize) -> String {
	let mut p = ChunkPrinter::new("web-1");
	let mut out = String::new();
	for frame in [&payload[..cut], &payload[cut..]] {
		if let Some(t) = p.stdout_text(frame) {
			out.push_str(&t);
		}
	}
	out
}

#[test]
fn a_character_split_between_frames_is_not_corrupted() {
	for ch in ["\u{e9}", "\u{20ac}", "\u{1f600}"] {
		let payload = format!("{ch}\n");
		for cut in 1..ch.len() {
			let got = printed(payload.as_bytes(), cut);
			assert!(
				!got.contains('\u{fffd}'),
				"{ch:?} cut at {cut} was corrupted: {got:?}"
			);
			// The first frame holds only part of the character, so the tag is
			// printed once, with the text it belongs to.
			assert_eq!(got, format!("web-1 | {payload}"));
		}
	}
}

#[test]
fn a_frame_with_only_the_start_of_a_character_prints_nothing_yet() {
	let mut p = ChunkPrinter::new("web-1");
	assert_eq!(p.stdout_text(&"\u{20ac}".as_bytes()[..2]), None);
	assert_eq!(
		p.stdout_text(&"\u{20ac}".as_bytes()[2..]).as_deref(),
		Some("web-1 | \u{20ac}")
	);
}

#[test]
fn the_two_channels_do_not_share_a_half_character() {
	let euro = "\u{20ac}".as_bytes();
	let mut p = ChunkPrinter::new("web-1");
	assert_eq!(p.stdout_text(&euro[..1]), None);
	// Stderr traffic in between must not complete or break the stdout tail.
	assert_eq!(p.stderr_text(b"e\n").as_deref(), Some("web-1 | e\n"));
	assert_eq!(
		p.stdout_text(&euro[1..]).as_deref(),
		Some("web-1 | \u{20ac}")
	);
}

#[test]
fn genuinely_invalid_bytes_keep_the_replacement_character() {
	let mut p = ChunkPrinter::new("web-1");
	assert_eq!(
		p.stdout_text(b"a\xffb\n").as_deref(),
		Some("web-1 | a\u{fffd}b\n")
	);
}
