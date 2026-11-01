//! Upload the context and consume build output through completion.

use std::io::IsTerminal;
use std::path::PathBuf;

use bytes::Bytes;
use futures_util::StreamExt;

use crate::compose::types::{BuildConfig, Service};
use crate::error::{ComposeError, Result};
use crate::libpod::types::image::BuildOutput;

use super::super::request::{context_body, multipart};
use super::super::{BodyPlan, BuildOptions, Engine};
use super::capture::BuildCapture;
use super::steps::{parse_image_id_line, BuildStreamProgress};
use super::upload;

pub(super) struct PreparedBuild {
	pub(super) body: BodyPlan,
	pub(super) additional: Vec<(String, PathBuf)>,
	pub(super) path: String,
	pub(super) tag: String,
	pub(super) wire_tag: String,
}

impl Engine {
	pub(super) async fn run_service_build(
		&self,
		service_name: &str,
		service: &Service,
		build: &BuildConfig,
		opts: &BuildOptions,
		prepared: PreparedBuild,
	) -> Result<()> {
		let PreparedBuild {
			body: body_plan,
			additional: additional_contexts,
			path,
			tag,
			wire_tag,
		} = prepared;
		let mut producer = None;
		let resp = match body_plan {
			BodyPlan::Empty if additional_contexts.is_empty() => self
				.client
				.post_bytes_stream(&path, Bytes::new(), "application/x-tar")
				.await
				.map_err(ComposeError::Podman)?,
			BodyPlan::Empty => {
				// The daemon fetches the main context itself, but the local
				// additional contexts are the client's to send: each one
				// travels in a part of the body.
				let boundary = multipart::boundary();
				let (writer, body) =
					multipart::multipart_body(boundary.clone(), None, additional_contexts);
				producer = Some(writer);
				let content_type = format!("multipart/form-data; boundary={boundary}");
				let sent = self
					.client
					.post_stream_body(&path, body, &content_type)
					.await;
				match sent {
					Ok(resp) => resp,
					Err(e) => {
						let _ = upload::finish_upload(producer).await;
						return Err(ComposeError::Podman(e));
					}
				}
			}
			BodyPlan::Stream {
				context,
				source,
				secrets,
			} if additional_contexts.is_empty() => {
				// A response head can arrive before the request body is sent.
				// Keep the producer until response polling has driven the HTTP
				// connection, then drop the body before joining or cancelling it.
				let (writer, body) = context_body(context, source, secrets);
				producer = Some(writer);
				let sent = self
					.client
					.post_stream_body(&path, body, "application/x-tar")
					.await;
				match sent {
					Ok(resp) => resp,
					Err(e) => {
						let _ = upload::finish_upload(producer).await;
						return Err(ComposeError::Podman(e));
					}
				}
			}
			BodyPlan::Stream {
				context,
				source,
				secrets,
			} => {
				// The endpoint reads the main context from the `MainContext` part and
				// each local additional context from a part of its own, so the body is
				// a multipart document. Same upload discipline as above: the producer
				// is kept until the response has been polled, then stopped.
				let boundary = multipart::boundary();
				let main = Some((context, source, secrets));
				let (writer, body) =
					multipart::multipart_body(boundary.clone(), main, additional_contexts);
				producer = Some(writer);
				let content_type = format!("multipart/form-data; boundary={boundary}");
				let sent = self
					.client
					.post_stream_body(&path, body, &content_type)
					.await;
				match sent {
					Ok(resp) => resp,
					Err(e) => {
						let _ = upload::finish_upload(producer).await;
						return Err(ComposeError::Podman(e));
					}
				}
			}
		};
		let mut stream = crate::libpod::parse_json_lines::<BuildOutput, _>(resp.into_body());

		// Open the board row for this image before the first `STEP` line, so
		// the row's `Building` verb is what the reader sees while the stream
		// arrives. `progress::start` is a no-op when the board already had
		// the row seeded (`build_all_with_options` did this for a standalone
		// `build`), and inserts it before the service's container row when
		// `up --build` calls into a board the `up` pass opened. Either way,
		// the row ends up with the right identifier, in the right position
		// and with a working verb (#1681).
		if !opts.quiet {
			let first_container = self.replica_names(service_name, service).into_iter().next();
			match first_container {
				Some(container_name) => crate::ui::progress::start_anchored(
					"Image",
					&tag,
					"Building",
					Some("Container"),
					Some(&container_name),
				),
				None => crate::ui::progress::start("Image", &tag, "Building"),
			}
		}

		// Keep only a bounded tail for terminal failure replay. In a pipe,
		// note_for already printed the output; quiet builds need no replay.
		let mut capture = BuildCapture::new(!opts.quiet && std::io::stderr().is_terminal());
		// Track the last image id the stream emitted, so the success path
		// can put it on stdout when stdout is not a terminal. Buildah
		// closes with a `--> sha256:<64-hex>` line, which is what a script
		// capturing stdout wants; on a terminal it is dropped (the row
		// already says it landed). `None` until the first matching line.
		let mut last_image_id: Option<String> = None;
		let mut progress = BuildStreamProgress::new();

		let stream_result = loop {
			let Some(result) = stream.next().await else {
				break Ok(());
			};
			match result {
				Ok(output) => {
					if !output.stream.is_empty() {
						let trimmed = output.stream.trim_end().to_string();
						if !opts.quiet && !trimmed.is_empty() {
							// `STEP n/m:` lines advance the row verb. Every
							// other line is a tail note, painted dimmed under
							// the row on a terminal and prefixed on stderr in
							// a pipe.
							if let Some(verb) = progress.observe(&trimmed) {
								crate::ui::progress::start("Image", &tag, &verb);
							}
							// The image id line is the one carry-over from
							// today that a script reading stdout needs. It
							// goes to notes/live as any other line, and is
							// additionally stashed so the success path can
							// echo it to stdout when stdout is not a tty.
							if let Some(id) = parse_image_id_line(&trimmed) {
								last_image_id = Some(id);
							}
							crate::ui::progress::note_for("Image", &tag, &trimmed);
						}
						capture.push(trimmed);
					}
					if let Some(err) = output.error_detail.and_then(|e| e.message) {
						break Err(ComposeError::Build(err));
					}
					if let Some(err) = output.error {
						if !err.is_empty() {
							break Err(ComposeError::Build(err));
						}
					}
				}
				Err(e) => break Err(ComposeError::Podman(e)),
			}
		};
		// DrivenBody owns the connection and the request's channel receiver.
		// Release both before waiting for a writer stopped by an early reply.
		drop(stream);
		let produced = upload::finish_upload(producer).await;
		match stream_result {
			Err(ComposeError::Build(err)) => {
				return Err(self
					.fail_build(&tag, err, capture.into_lines(), opts.quiet)
					.await);
			}
			Err(err) => return Err(err),
			Ok(()) => produced?,
		}

		// Success: close the row with `Built`. The trailing image id goes to
		// stdout only when stdout is not a terminal, so a script reading
		// `podup build | awk '{print $1}'` can still pluck the id; on a
		// terminal the row says it landed and the id would only be noise.
		if !opts.quiet {
			crate::ui::progress_line("Image", &tag, "Built");
		}
		if !std::io::stdout().is_terminal() {
			let resolved = match last_image_id {
				Some(id) => Some(id),
				None => match self.image_id(&tag).await {
					Ok(Some(id)) => Some(id),
					_ => None,
				},
			};
			if let Some(id) = resolved {
				use std::io::Write;
				let _ = writeln!(std::io::stdout(), "{id}");
			}
		}

		self.apply_extra_tags(build, &tag, &wire_tag).await?;
		Ok(())
	}
}
