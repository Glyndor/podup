use super::{incomplete_tail_len, Utf8Chunks};

const SAMPLES: [&str; 3] = ["\u{e9}", "\u{20ac}", "\u{1f600}"];

fn decode_in_pieces(pieces: &[&[u8]]) -> String {
	let mut d = Utf8Chunks::default();
	let mut out = String::new();
	for p in pieces {
		out.push_str(&d.push(p));
	}
	out.push_str(&d.finish());
	out
}

#[test]
fn a_character_split_at_every_position_comes_out_whole() {
	for ch in SAMPLES {
		let text = format!("a{ch}b");
		let bytes = text.as_bytes();
		for cut in 0..=bytes.len() {
			let got = decode_in_pieces(&[&bytes[..cut], &bytes[cut..]]);
			assert_eq!(got, text, "{ch:?} split at byte {cut}");
		}
	}
}

#[test]
fn a_character_fed_one_byte_at_a_time_comes_out_whole() {
	for ch in SAMPLES {
		let text = format!("x{ch}{ch}y");
		let pieces: Vec<&[u8]> = text.as_bytes().chunks(1).collect();
		assert_eq!(decode_in_pieces(&pieces), text);
	}
}

#[test]
fn invalid_bytes_still_become_replacement_characters() {
	let cases: [&[u8]; 4] = [b"a\xffb", b"a\x80b", b"a\xc3(b", b"a\xe2\x82(b"];
	for bytes in cases {
		let want = String::from_utf8_lossy(bytes).into_owned();
		assert!(want.contains('\u{fffd}'));
		assert_eq!(decode_in_pieces(&[bytes]), want);
		for cut in 0..=bytes.len() {
			let got = decode_in_pieces(&[&bytes[..cut], &bytes[cut..]]);
			assert_eq!(got, want, "{bytes:?} split at {cut}");
		}
	}
}

#[test]
fn a_truncated_character_at_the_end_is_reported_on_finish() {
	let euro = "\u{20ac}".as_bytes();
	let mut d = Utf8Chunks::default();
	assert_eq!(d.push(&euro[..2]), "");
	assert_eq!(d.finish(), "\u{fffd}");
	assert_eq!(d.finish(), "", "finish drains the carry");
}

#[test]
fn incomplete_tail_only_counts_what_could_still_be_completed() {
	assert_eq!(incomplete_tail_len(b""), 0);
	assert_eq!(incomplete_tail_len(b"abc"), 0);
	assert_eq!(incomplete_tail_len(&"\u{20ac}".as_bytes()[..1]), 1);
	assert_eq!(incomplete_tail_len(&"\u{20ac}".as_bytes()[..2]), 2);
	assert_eq!(incomplete_tail_len("\u{20ac}".as_bytes()), 0);
	assert_eq!(incomplete_tail_len(&"\u{1f600}".as_bytes()[..3]), 3);
	// Never completable: a lone continuation byte, an overlong lead byte, a
	// lead byte followed by a non-continuation.
	assert_eq!(incomplete_tail_len(b"\x80"), 0);
	assert_eq!(incomplete_tail_len(b"\xc0"), 0);
	assert_eq!(incomplete_tail_len(b"\xe2("), 0);
}
