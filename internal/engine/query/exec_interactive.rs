//! Interactive `exec`: a pseudo-terminal and a live stdin, on Unix.
//!
//! `podup exec -it db psql` is the most-used shell-into-a-container habit in
//! compose, and it did not work: podup allocated no TTY and attached no stdin,
//! so `-i` set a flag nothing read and `-T` disabled something that never
//! existed (#1079).
//!
//! Unix only for now. podup reaches `podman machine` over a named pipe on
//! Windows, and both raw mode and resize events there are a different API — two
//! implementations, not one behind a `cfg`. The non-interactive path is
//! unchanged everywhere, so `podup exec` in a script behaves exactly as before
//! on every platform.

#![cfg(unix)]

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::error::{ComposeError, Result};
use crate::libpod::{urlencoded, API_PREFIX};

use super::terminal::{window_size, RawMode};
use super::Engine;

impl Engine {
	/// Run an exec session with a pty and a live stdin.
	///
	/// The terminal is restored by `RawMode`'s `Drop`, so every exit path —
	/// command exits, socket dies, `?` on an unrelated error, panic — leaves the
	/// caller's shell usable. That is the one thing this must not get wrong: a
	/// terminal left raw has no echo and no line discipline, and the user cannot
	/// even see what they type to fix it.
	pub(super) async fn exec_interactive(&self, exec_id: &str) -> Result<()> {
		let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(exec_id));
		let body = br#"{"Detach":false,"Tty":true}"#;
		let hijacked = self
			.client
			.post_hijack(&start_path, body)
			.await
			.map_err(ComposeError::Podman)?;

		// Raw mode only *after* the exec is known to have started. Enabling it
		// first would leave a terminal unusable behind a 404.
		let _raw = RawMode::enable();

		// Size the pty now, not before the start: there is no pty to resize until
		// the exec has begun, and libpod rejects the call — silently, since a
		// failed resize is only cosmetic. Sizing it here means a full-screen
		// program draws correctly from its first frame rather than at libpod's
		// 80x24 default until the window happens to change.
		if let Some((rows, cols)) = window_size() {
			self.resize_exec(exec_id, rows, cols).await;
		}

		let (mut server_read, mut server_write) = tokio::io::split(hijacked.stream);
		let mut stdin = tokio::io::stdin();
		let mut stdout = tokio::io::stdout();

		// Follow the window. libpod has no way to learn the caller's terminal
		// size on its own, so every SIGWINCH has to be forwarded or a resized
		// window leaves the remote program drawing at the old geometry.
		let mut winch =
			tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
				.map_err(ComposeError::Io)?;

		let mut to_server = [0u8; 8 * 1024];
		let mut from_server = [0u8; 32 * 1024];

		loop {
			tokio::select! {
				// Output first: a biased select would starve it behind a caller
				// holding the keyboard down, and seeing the command's output is
				// the point of being attached.
				read = server_read.read(&mut from_server) => {
					match read {
						Ok(0) => break,
						Ok(n) => {
							stdout.write_all(&from_server[..n]).await.map_err(ComposeError::Io)?;
							stdout.flush().await.map_err(ComposeError::Io)?;
						}
						// The command exiting closes this side; that is the
						// ordinary end of an interactive session, not a failure.
						Err(_) => break,
					}
				}
				read = stdin.read(&mut to_server) => {
					match read {
						// Ctrl-D / a closed stdin: stop writing but keep reading,
						// so the command can finish and flush. Shutting the write
						// half is what tells it EOF.
						Ok(0) => {
							let _ = server_write.shutdown().await;
						}
						Ok(n) => {
							if server_write.write_all(&to_server[..n]).await.is_err() {
								break;
							}
							if server_write.flush().await.is_err() {
								break;
							}
						}
						Err(_) => break,
					}
				}
				_ = winch.recv() => {
					if let Some((rows, cols)) = window_size() {
						self.resize_exec(exec_id, rows, cols).await;
					}
				}
			}
		}

		Ok(())
	}

	/// Tell libpod the pty's new size. Best-effort: a failed resize makes the
	/// remote program draw at the wrong geometry, which is a cosmetic problem,
	/// and turning it into a hard error would kill a working session over a
	/// window drag.
	async fn resize_exec(&self, exec_id: &str, rows: u16, cols: u16) {
		let path = format!(
			"{API_PREFIX}/exec/{}/resize?h={rows}&w={cols}",
			urlencoded(exec_id),
		);
		if let Err(e) = self.client.post_empty_ok(&path).await {
			tracing::debug!("exec resize to {rows}x{cols} failed: {e}");
		}
	}
}
