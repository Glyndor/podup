//! A session that carries the caller's standard input and no pseudo-terminal.
//!
//! `podup exec -T db psql < dump.sql` has to feed the file to the command.
//! That needs two things the plain streaming path does not have: the session
//! has to be created with its stdin attached, and the connection has to stay
//! open in both directions, because the caller's input goes up the same
//! connection the command's output comes down.
//!
//! Measured against Podman 5.7.0: `POST /libpod/exec/{id}/start` with
//! `{"Detach":false,"Tty":false}` on a session created with
//! `AttachStdin: true` hijacks the connection. What the client writes after
//! the response head is the command's stdin, and what the server writes back
//! is the command's output, multiplexed in 8-byte frames (type 1 = stdout,
//! type 2 = stderr) because no pseudo-terminal merges the two channels.

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{ComposeError, Result};
use crate::libpod::types::stream::{parse_frame, MAX_STREAM_BUF};
use crate::libpod::{LogOutput, PodmanError};

use super::output::{frame_action, FrameAction};

/// Pump a session that has the caller's stdin attached.
///
/// `session` is the hijacked libpod connection: bytes read from it are the
/// command's output, bytes written to it are the command's stdin. `stdin`
/// is the caller's input, forwarded until it ends; the write half is then
/// shut down so the command sees EOF and can finish. Without that half-close
/// the command would wait for input that never comes.
///
/// The output is demultiplexed here and the payload bytes are handed to the
/// caller untouched: a command that writes a compressed archive to stdout
/// gets that archive, not a transcription of it.
pub(super) async fn pump_piped<S, I, O, E>(
	session: S,
	mut stdin: I,
	mut stdout: O,
	mut stderr: E,
) -> Result<()>
where
	S: AsyncRead + AsyncWrite + Unpin,
	I: AsyncRead + Unpin,
	O: AsyncWrite + Unpin,
	E: AsyncWrite + Unpin,
{
	let (mut session_read, mut session_write) = tokio::io::split(session);
	// Each direction owns its entire read/write loop. An await inside one
	// pump must not prevent the other from relieving backpressure.
	let input = async {
		let mut to_session = [0u8; 8 * 1024];
		loop {
			match stdin.read(&mut to_session).await {
				Ok(0) => {
					let _ = session_write.shutdown().await;
					break;
				}
				Ok(n) => {
					if session_write.write_all(&to_session[..n]).await.is_err() {
						// A command may finish without consuming all its input.
						tracing::debug!("exec: the command stopped reading its stdin");
						break;
					}
				}
				Err(_) => {
					tracing::debug!("exec: reading the caller's stdin failed");
					break;
				}
			}
		}
	};
	let output = async {
		let mut buffered = BytesMut::new();
		let mut from_session = [0u8; 32 * 1024];
		loop {
			let n = session_read
				.read(&mut from_session)
				.await
				.map_err(|e| ComposeError::Podman(PodmanError::Connect(e)))?;
			if n == 0 {
				if !buffered.is_empty() {
					return Err(ComposeError::Podman(PodmanError::StreamEndedEarly));
				}
				break;
			}
			// Bound daemon-controlled buffering while a frame is incomplete;
			// the extra eight bytes are its header.
			if buffered.len() + n > MAX_STREAM_BUF + 8 {
				return Err(ComposeError::Podman(PodmanError::StreamTooLarge));
			}
			buffered.extend_from_slice(&from_session[..n]);
			while let Some((stream_type, payload)) = parse_frame(&mut buffered)? {
				let frame = match stream_type {
					1 => LogOutput::StdOut { message: payload },
					2 => LogOutput::StdErr { message: payload },
					_ => continue,
				};
				match frame_action(frame) {
					FrameAction::StdOut(bytes) => {
						stdout.write_all(&bytes).await.map_err(ComposeError::Io)?
					}
					FrameAction::StdErr(bytes) => {
						stderr.write_all(&bytes).await.map_err(ComposeError::Io)?
					}
					FrameAction::Drop => {}
				}
			}
		}
		// Tokio's stdio writers may finish write_all before the blocking
		// write completes. Flush both channels and surface deferred failures.
		stdout.flush().await.map_err(ComposeError::Io)?;
		stderr.flush().await.map_err(ComposeError::Io)?;
		Ok(())
	};
	tokio::pin!(output);
	// EOF on stdin only half-closes the session; keep draining output. EOF
	// or failure on output ends the session and drops any blocked input pump.
	tokio::select! {
		result = &mut output => result,
		() = input => output.await,
	}
}

#[cfg(test)]
#[path = "piped_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "piped_regression_tests.rs"]
mod regression_tests;
