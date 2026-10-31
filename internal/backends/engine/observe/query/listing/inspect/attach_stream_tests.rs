//! The `attach` stream's frames, byte for byte.

use bytes::Bytes;

use super::write_attach_frame;
use crate::libpod::LogOutput;

/// The payload is bytes: what the container printed is what the caller gets.
/// Each frame used to be converted on its own, so a byte that is not UTF-8
/// arrived as a replacement character.
#[test]
fn a_frame_is_written_byte_for_byte() {
	let mut out: Vec<u8> = Vec::new();
	let mut err: Vec<u8> = Vec::new();
	write_attach_frame(
		&mut out,
		&mut err,
		LogOutput::StdOut {
			message: Bytes::from_static(b"\xff\xfe"),
		},
	)
	.expect("the write succeeds");
	assert_eq!(out, b"\xff\xfe", "the bytes arrive unchanged");
	assert!(err.is_empty(), "a stdout frame does not go to stderr");
}

/// A character that arrives split across two frames is one character. The
/// frames are written one after the other, so the bytes concatenate and the
/// character is whole; converted frame by frame it became two replacement
/// characters.
#[test]
fn a_character_split_across_two_frames_arrives_whole() {
	let euro = "€".as_bytes();
	let mut out: Vec<u8> = Vec::new();
	let mut err: Vec<u8> = Vec::new();
	write_attach_frame(
		&mut out,
		&mut err,
		LogOutput::StdOut {
			message: Bytes::copy_from_slice(&euro[..1]),
		},
	)
	.expect("the write succeeds");
	write_attach_frame(
		&mut out,
		&mut err,
		LogOutput::StdOut {
			message: Bytes::copy_from_slice(&euro[1..]),
		},
	)
	.expect("the write succeeds");
	assert_eq!(out, euro, "the character arrives whole");
}

/// stderr goes to stderr.
#[test]
fn a_stderr_frame_goes_to_stderr() {
	let mut out: Vec<u8> = Vec::new();
	let mut err: Vec<u8> = Vec::new();
	write_attach_frame(
		&mut out,
		&mut err,
		LogOutput::StdErr {
			message: Bytes::from_static(b"boom"),
		},
	)
	.expect("the write succeeds");
	assert!(out.is_empty(), "a stderr frame does not go to stdout");
	assert_eq!(err, b"boom");
}
