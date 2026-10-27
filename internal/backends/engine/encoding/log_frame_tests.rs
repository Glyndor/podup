//! `write_frame` hands the caller's output to the writer unchanged.
//!
//! What a command printed is bytes, not text. A `run`/`exec` session that
//! pipes a compressed archive to stdout, or a program that writes the two
//! halves of one multi-byte character in two separate writes, has to get
//! those bytes out the way they arrived. Decoding them on the way out
//! replaced every byte that is not valid UTF-8 with U+FFFD and turned every
//! character that arrived split across two frames into two replacement
//! characters, so the output was not the output.

use super::write_frame;

/// Bytes that are not valid UTF-8 must reach the writer as they arrived.
#[test]
fn write_frame_writes_bytes_that_are_not_valid_utf8_unchanged() {
	let mut buf = Vec::new();
	write_frame(&mut buf, b"ok\xFF!\n").unwrap();
	assert_eq!(buf, b"ok\xFF!\n");
}

/// A character split across two frames must come out whole. Each half on its
/// own is not valid UTF-8, so a decoder that treats every frame as a
/// complete text replaces both halves and the character is lost.
#[test]
fn write_frame_keeps_a_character_split_across_two_frames() {
	let bytes = "€".as_bytes();
	assert_eq!(bytes.len(), 3, "the euro sign is three bytes");

	let mut buf = Vec::new();
	write_frame(&mut buf, &bytes[..1]).unwrap();
	write_frame(&mut buf, &bytes[1..]).unwrap();
	assert_eq!(buf, bytes);
}

/// The same, for a whole binary blob: a session that streams a file to
/// stdout must deliver the file, not a transcription of it.
#[test]
fn write_frame_writes_a_binary_blob_unchanged() {
	let blob: Vec<u8> = (0u8..=255).collect();
	let mut buf = Vec::new();
	write_frame(&mut buf, &blob).unwrap();
	assert_eq!(buf, blob);
}
