/// Options for
/// [`Engine::pull_services_with_options`](crate::engine::Engine::pull_services_with_options),
/// mirroring `docker compose pull` flags. The `--policy` override is carried on
/// the engine (see
/// [`Engine::with_up_overrides`](crate::engine::Engine::with_up_overrides)), not here.
///
/// `#[non_exhaustive]` since 4.0.0, so a new flag can be added in a minor
/// release without breaking every external caller that built the struct with
/// a literal. Construct it via [`PullOptions::new`] or the `with_*` builders
/// below; a struct literal is refused outside this crate, which is what buys
/// the room to grow.
#[derive(Default)]
#[non_exhaustive]
pub struct PullOptions {
	/// Warn and continue instead of aborting on the first failure,
	/// `--ignore-pull-failures`.
	pub ignore_failures: bool,
	/// Also pull each named service's transitive `depends_on`, `--include-deps`.
	pub include_deps: bool,
}

impl PullOptions {
	/// Every `docker compose pull` flag, in CLI order. A constructor rather
	/// than a struct literal because the type is `#[non_exhaustive]`, so the
	/// next flag to land is not a breaking change for anyone building one.
	pub fn new(ignore_failures: bool, include_deps: bool) -> Self {
		Self {
			ignore_failures,
			include_deps,
		}
	}

	/// Warn and continue instead of aborting on the first failure,
	/// `--ignore-pull-failures`. Builder-style.
	#[must_use]
	pub fn with_ignore_failures(mut self, ignore_failures: bool) -> Self {
		self.ignore_failures = ignore_failures;
		self
	}
}
