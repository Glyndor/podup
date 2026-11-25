//! Signals sent to `podup exec` reach the command it runs.
//!
//! `podup exec` is a client: the command runs in the container, in a process
//! podup does not own. A SIGTERM or SIGINT sent to podup used to end podup and
//! nothing else, with status 143 or 130, and the command went on running with
//! nobody reading its output. `docker compose exec` does the same (measured
//! against the same Podman 6.2.0 socket: status 130 and the command still
//! running in the container), so this is not parity but the behaviour a
//! script that stops an `exec` needs.
//!
//! libpod has a route for it, `POST /libpod/exec/{id}/kill?signal=NAME`
//! (measured on Podman 6.2.0: 200, and a command that traps TERM ran its trap
//! and exited with its own status). The relay forwards SIGTERM, SIGINT and
//! SIGHUP there and keeps waiting for the session to end, so the exit status
//! podup returns is the command's, read from the exec as for any other run.
//! The command decides what a signal means; one that ignores it keeps running
//! and so does `podup exec`, as it would in a shell.
//!
//! In a pseudo-terminal session Ctrl-C reaches the command as a keystroke and
//! never as a signal to podup; the relay matters there for SIGTERM and SIGHUP.

use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::error::Result;
use crate::libpod::{urlencoded, API_PREFIX};

use crate::backends::engine::observe::query::Engine;

/// A signal podup passes on to the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Relayed {
	Term,
	Int,
	Hup,
}

impl Relayed {
	/// The name the kill route takes.
	pub(super) fn name(self) -> &'static str {
		match self {
			Self::Term => "SIGTERM",
			Self::Int => "SIGINT",
			Self::Hup => "SIGHUP",
		}
	}
}

/// How long a signal that arrives before the command has started is held, and
/// how often it is tried again. The exec exists once it is created but cannot
/// be signalled until the start request has been served.
const RETRY_EVERY: Duration = Duration::from_millis(100);
const RETRIES: u32 = 30;

/// The signals podup has received, for as long as the guard lives.
pub(super) struct Listener {
	pub(super) signals: mpsc::UnboundedReceiver<Relayed>,
	task: tokio::task::JoinHandle<()>,
}

impl Drop for Listener {
	fn drop(&mut self) {
		self.task.abort();
	}
}

/// Start listening for SIGTERM, SIGINT and SIGHUP. `None` when the process
/// cannot register for them, which leaves the signals with their default
/// effect.
#[cfg(unix)]
pub(super) fn listen() -> Option<Listener> {
	use tokio::signal::unix::{signal, SignalKind};
	let mut term = signal(SignalKind::terminate()).ok()?;
	let mut int = signal(SignalKind::interrupt()).ok()?;
	let mut hup = signal(SignalKind::hangup()).ok()?;
	let (tx, signals) = mpsc::unbounded_channel();
	let task = tokio::spawn(async move {
		loop {
			let which = tokio::select! {
				_ = term.recv() => Relayed::Term,
				_ = int.recv() => Relayed::Int,
				_ = hup.recv() => Relayed::Hup,
			};
			if tx.send(which).is_err() {
				return;
			}
		}
	});
	Some(Listener { signals, task })
}

/// A console has Ctrl-C and nothing else to listen for.
#[cfg(not(unix))]
pub(super) fn listen() -> Option<Listener> {
	let (tx, signals) = mpsc::unbounded_channel();
	let task = tokio::spawn(async move {
		while tokio::signal::ctrl_c().await.is_ok() {
			if tx.send(Relayed::Int).is_err() {
				return;
			}
		}
	});
	Some(Listener { signals, task })
}

impl Engine {
	/// Run an exec session with signals relayed to the command.
	pub(super) async fn run_session<F>(&self, exec_id: &str, session: F) -> Result<()>
	where
		F: Future<Output = Result<()>>,
	{
		match listen() {
			Some(mut listener) => {
				self.relay_signals(exec_id, &mut listener.signals, session)
					.await
			}
			None => session.await,
		}
	}

	/// Drive `session` to its end, passing every signal that arrives to the exec.
	pub(super) async fn relay_signals<F>(
		&self,
		exec_id: &str,
		signals: &mut mpsc::UnboundedReceiver<Relayed>,
		session: F,
	) -> Result<()>
	where
		F: Future<Output = Result<()>>,
	{
		tokio::pin!(session);
		let mut pending: Option<(Relayed, u32)> = None;
		loop {
			tokio::select! {
				result = &mut session => return result,
				Some(which) = signals.recv() => pending = Some((which, 0)),
				() = tokio::time::sleep(RETRY_EVERY), if pending.is_some() => {}
			}
			let Some((which, tries)) = pending.take() else {
				continue;
			};
			if !self.signal_exec(exec_id, which).await && tries < RETRIES {
				pending = Some((which, tries + 1));
			}
		}
	}

	/// Send `which` to the exec. `false` when it was not delivered because the
	/// exec is not running yet, so the caller can try again.
	async fn signal_exec(&self, exec_id: &str, which: Relayed) -> bool {
		let path = format!(
			"{API_PREFIX}/exec/{}/kill?signal={}",
			urlencoded(exec_id),
			which.name()
		);
		match self.client.post_empty_ok(&path).await {
			Ok(()) => true,
			Err(e) if e.is_status(409) || e.is_status(404) => {
				tracing::debug!("exec {exec_id}: {} not delivered yet ({e})", which.name());
				false
			}
			Err(e) => {
				tracing::debug!("exec {exec_id}: {} not delivered ({e})", which.name());
				true
			}
		}
	}
}

#[cfg(all(test, unix))]
#[path = "relay_tests.rs"]
mod tests;
