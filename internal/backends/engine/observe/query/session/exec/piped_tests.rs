//! The session that carries the caller's standard input.
//!
//! Two things have to hold for `exec -T db psql < dump.sql` to work: the
//! caller's bytes reach the command, and the command's output comes back
//! demultiplexed and unchanged.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::pump_piped;

/// One multiplexed stdout frame carrying `payload`.
fn stdout_frame(payload: &[u8]) -> Vec<u8> {
	let mut frame = vec![1u8, 0, 0, 0, 0, 0, 0, payload.len() as u8];
	frame.extend_from_slice(payload);
	frame
}

/// One multiplexed stderr frame carrying `payload`.
fn stderr_frame(payload: &[u8]) -> Vec<u8> {
	let mut frame = vec![2u8, 0, 0, 0, 0, 0, 0, payload.len() as u8];
	frame.extend_from_slice(payload);
	frame
}

/// The caller's input reaches the command, and the command's output comes
/// back on the caller's stdout and stderr.
#[tokio::test]
async fn the_callers_input_reaches_the_session_and_the_output_comes_back() {
	let (mut peer, session) = tokio::io::duplex(1024);
	let stdin = std::io::Cursor::new(b"SELECT 1;\n".to_vec());
	let mut stdout = Vec::new();
	let mut stderr = Vec::new();

	// The far end of the session: the command's output, then it waits for
	// the caller's input and ends the session once it has it.
	let peer_task = tokio::spawn(async move {
		peer.write_all(&stdout_frame(b"hello")).await.unwrap();
		peer.write_all(&stderr_frame(b"oops")).await.unwrap();
		let mut received = Vec::new();
		peer.read_to_end(&mut received).await.unwrap();
		peer.shutdown().await.unwrap();
		received
	});

	pump_piped(session, stdin, &mut stdout, &mut stderr)
		.await
		.expect("the session should end cleanly");

	assert_eq!(stdout, b"hello", "the stdout frame must reach stdout");
	assert_eq!(stderr, b"oops", "the stderr frame must reach stderr");
	assert_eq!(
		peer_task.await.unwrap(),
		b"SELECT 1;\n",
		"the caller's input must reach the session"
	);
}

/// The bytes are the command's bytes: a payload that is not valid UTF-8
/// arrives as it was written, and a character split across two frames comes
/// out whole.
#[tokio::test]
async fn the_output_arrives_byte_for_byte() {
	let (mut peer, session) = tokio::io::duplex(1024);
	let stdin = std::io::Cursor::new(Vec::new());
	let mut stdout = Vec::new();
	let mut stderr = Vec::new();

	let peer_task = tokio::spawn(async move {
		peer.write_all(&stdout_frame(b"ok\xFF")).await.unwrap();
		// The two halves of one character, in two frames.
		peer.write_all(&stdout_frame(&"€".as_bytes()[..1]))
			.await
			.unwrap();
		peer.write_all(&stdout_frame(&"€".as_bytes()[1..]))
			.await
			.unwrap();
		let mut received = Vec::new();
		peer.read_to_end(&mut received).await.unwrap();
		peer.shutdown().await.unwrap();
		received
	});

	pump_piped(session, stdin, &mut stdout, &mut stderr)
		.await
		.expect("the session should end cleanly");
	peer_task.await.unwrap();

	let mut expected = b"ok\xFF".to_vec();
	expected.extend_from_slice("€".as_bytes());
	assert_eq!(stdout, expected);
}
