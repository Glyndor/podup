//! Per-service build: build one `build:` block end-to-end.
//!
//! The dispatching loop in `mod.rs` calls into [`Engine::build_service`],
//! which is the URL/stream glue for one image: pick a body plan,
//! assemble the build query, drive the chunked response, paint the
//! board row. Each step is small enough that the whole thing lives
//! here. The body-plan decision lives in [`super::request::body_plan`]; the
//! `apply_extra_tags` follow-up lives in [`super::names::extra_tags`].

mod cache;
mod capture;
mod execution;
mod query;
mod steps;
mod upload;

use crate::compose::types::Service;
use crate::error::Result;
use crate::libpod::{urlencoded, API_PREFIX};

use self::execution::PreparedBuild;
use super::request::plan_build;
use super::{BodyPlan, BuildOptions, Engine};

impl Engine {
	pub(in crate::backends::engine) async fn build_service(
		&self,
		service_name: &str,
		service: &Service,
		file: &crate::compose::types::ComposeFile,
		opts: &BuildOptions,
	) -> Result<()> {
		let build = match &service.build {
			Some(b) => b,
			None => return Ok(()),
		};

		let context_str = build.context().to_string();
		// `tag` is the un-normalised form the user (or `primary_build_tag`'s
		// `<project>-<service>:latest` default) supplied. It is what every
		// print path carries - the board row `up` and `build` seed, the
		// `Building`/`Built` verbs, the `STEP n/m:` line prefix, the
		// `fail_build` error message, and the `apply_extra_tags` comparison.
		// Only the wire query (`t=` and `/images/{}/tag`) carries the
		// docker.io canonical form, computed just before the query string is
		// assembled below.
		let tag = super::primary_build_tag(
			&self.project,
			service_name,
			service.image.as_deref(),
			build.tags(),
		);

		let plan = plan_build(self, service_name, service, file, build, &context_str)?;
		let (mut qs, wire_tag) = self
			.service_build_query(
				service_name,
				build,
				opts,
				&tag,
				&plan.dockerfile,
				&plan.secrets,
			)
			.await?;
		if matches!(plan.body, BodyPlan::Empty) {
			qs.push_str(&format!("&remote={}", urlencoded(&context_str)));
		}
		let prepared = PreparedBuild {
			body: plan.body,
			additional: plan.additional,
			path: format!("{API_PREFIX}/build?{qs}"),
			tag,
			wire_tag,
		};
		self.run_service_build(service_name, service, build, opts, prepared)
			.await
	}
}

#[cfg(test)]
mod tests;

#[cfg(all(test, unix))]
mod board_tests;
#[cfg(all(test, unix))]
mod cgroup_hint_tests;
#[cfg(test)]
mod steps_tests;
