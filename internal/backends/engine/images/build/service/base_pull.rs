//! Whether a build refreshes its base images.

use crate::compose::types::Service;

use super::super::{BuildOptions, Engine};

impl Engine {
	/// `opts`, with `pull` set when the service asks for its base images to be
	/// refreshed: `pull_policy: always` on the service, or an `always` policy
	/// forced for the whole run. `build.pull` and `--pull` already reach the
	/// query on their own.
	pub(super) fn refreshing_opts(
		&self,
		service_name: &str,
		service: &Service,
		opts: &BuildOptions,
	) -> BuildOptions {
		let mut opts = opts.clone();
		let always = self
			.resolved_pull_policy(service_name, service)
			.is_ok_and(|policy| policy.libpod() == "always");
		opts.pull |= always;
		opts
	}
}
