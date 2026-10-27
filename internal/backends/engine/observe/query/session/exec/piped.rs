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
	let mut buffered = BytesMut::new();
	let mut to_session = [0u8; 8 * 1024];
	let mut from_session = [0u8; 32 * 1024];
	// The caller's stdin has ended (or the command stopped reading it); the
	// session keeps streaming its output either way.
	let mut stdin_done = false;
	loop {
		tokio::select! {
			read = session_read.read(&mut from_session) => {
				match read {
					// The command finished and the session ended with it.
					Ok(0) => break,
					Ok(n) => {
						// The same bound the multiplexed parser applies to an
						// HTTP body: it caps what a daemon-controlled stream
						// can make the client allocate while a frame is still
						// incomplete. The extra eight bytes are the frame
						// header, which may sit ahead of its payload.
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
					Err(e) => {
						return Err(ComposeError::Podman(PodmanError::Connect(e)));
					}
				}
			}
			read = stdin.read(&mut to_session), if !stdin_done => {
				match read {
					// The caller's input ended: the command sees EOF.
					Ok(0) => {
						stdin_done = true;
						let _ = session_write.shutdown().await;
					}
					Ok(n) => {
						if let Err(e) = session_write.write_all(&to_session[..n]).await {
							// The command stopped reading its input (it exited
							// without consuming it). That is not a failure of
							// the session: its output is still on the way.
							tracing::debug!("exec: the command stopped reading its stdin: {e}");
							stdin_done = true;
						}
					}
					Err(e) => {
						tracing::debug!("exec: reading the caller's stdin failed: {e}");
						stdin_done = true;
					}
				}
			}
		}
	}
	Ok(())
}

#[cfg(test)]
#[path = "piped_tests.rs"]
mod tests;
