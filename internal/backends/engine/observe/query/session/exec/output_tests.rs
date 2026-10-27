//! An exec session's output reaches the caller byte for byte.
//!
//! The frames a session produces carry bytes, and the writer is the last
//! place they can be altered. A frame whose payload is not valid UTF-8 must
//! not be replaced with U+FFFD, and two frames that carry the two halves of
//! one character must come out as that character.

use bytes::Bytes;

use super::write_exec_frame;
use crate::libpod::LogOutput;

/// Bytes that are not valid UTF-8 must reach the writer as they arrived.
#[test]
fn stdout_frame_bytes_are_written_unchanged() {
	let mut out = Vec::new();
	let mut err = Vec::new();
	write_exec_frame(
		&mut out,
		&mut err,
		LogOutput::StdOut {
			message: Bytes::from_static(b"ok\xFF!\n"),
		},
	)
	.unwrap();
	assert_eq!(out, b"ok\xFF!\n");
	assert!(err.is_empty(), "a stdout frame must not go to stderr");
}

/// The same for the stderr channel.
#[test]
fn stderr_frame_bytes_are_written_unchanged() {
	let mut out = Vec::new();
	let mut err = Vec::new();
	write_exec_frame(
		&mut out,
		&mut err,
		LogOutput::StdErr {
			message: Bytes::from_static(b"bad\xFE\n"),
		},
	)
	.unwrap();
	assert_eq!(err, b"bad\xFE\n");
	assert!(out.is_empty(), "a stderr frame must not go to stdout");
}

/// A character split across two frames must come out whole.
#[test]
fn a_character_split_across_two_frames_comes_out_whole() {
	let bytes = "€".as_bytes();
	let mut out = Vec::new();
	let mut err = Vec::new();
	write_exec_frame(
		&mut out,
		&mut err,
		LogOutput::StdOut {
			message: Bytes::copy_from_slice(&bytes[..1]),
		},
	)
	.unwrap();
	write_exec_frame(
		&mut out,
		&mut err,
		LogOutput::StdOut {
			message: Bytes::copy_from_slice(&bytes[1..]),
		},
	)
	.unwrap();
	assert_eq!(out, bytes);
}

/// The teardown noise an OCI exec launch failure emits is still dropped, so
/// only the real diagnostic shows.
#[test]
fn the_teardown_noise_frame_is_dropped() {
	let mut out = Vec::new();
	let mut err = Vec::new();
	write_exec_frame(
		&mut out,
		&mut err,
		LogOutput::StdErr {
			message: Bytes::from_static(
				b"read unixpacket @->/run/user/1000/podman/podman.sock: connection reset by peer\n",
			),
		},
	)
	.unwrap();
	assert!(out.is_empty());
	assert!(
		err.is_empty(),
		"the teardown noise must not reach the caller"
	);
}
