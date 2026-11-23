//! The plain `exec` session: no pseudo-terminal and no stdin, the command's
//! output streamed back as multiplexed frames.

use std::io::Write;

use futures_util::StreamExt;

use crate::error::{ComposeError, Result};
use crate::libpod::types::exec::ExecStartConfig;
use crate::libpod::{urlencoded, API_PREFIX};

use crate::backends::engine::observe::query::Engine;

use super::{map_exec_start_err, output, ExecOptions, EXEC_START_TIMEOUT};

impl Engine {
	/// Start the exec and copy its output to stdout and stderr until it ends.
	pub(super) async fn exec_streamed(&self, exec_id: &str, opts: &ExecOptions) -> Result<()> {
		let start_cfg = ExecStartConfig {
			detach: false,
			tty: false,
		};
		let start_path = format!("{API_PREFIX}/exec/{}/start", urlencoded(exec_id));
		let start_resp = self
			.client
			.post_json_stream_within(&start_path, &start_cfg, Some(EXEC_START_TIMEOUT))
			.await
			.map_err(|e| map_exec_start_err(e, opts))?;
		let mut stream = crate::libpod::parse_multiplexed(start_resp.into_body());

		// Lock stdout once for the whole stream instead of re-acquiring the lock
		// (and issuing a syscall) per frame; stdout is ours exclusively on this
		// path. stderr is locked per frame because the tracing subscriber also
		// writes there: holding its lock across the await loop would starve
		// concurrent log emissions. Flush after each frame so exec streams
		// promptly.
		let mut out = std::io::stdout().lock();
		while let Some(msg) = stream.next().await {
			let frame = msg.map_err(ComposeError::Podman)?;
			let mut err = std::io::stderr().lock();
			let _ = output::write_exec_frame(&mut out, &mut err, frame);
			let _ = out.flush();
		}
		Ok(())
	}
}
