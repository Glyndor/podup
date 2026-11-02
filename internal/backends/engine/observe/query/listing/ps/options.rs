//! The public option structs for `ps` (`PsOptions`, `PsDisplayOptions`,
//! `PsFilterOptions`) and their builder methods. Pure published surface.
//!
//! Held apart from `mod.rs` because every field is something an external
//! caller sets and every `with_*` is part of the API the crate promises not
//! to break.

/// Options for [`Engine::ps_with_options`](crate::Engine::ps_with_options), mirroring `docker compose ps`.
///
/// `#[non_exhaustive]` since 4.0.0, so a new flag can be added in a minor
/// release without breaking every external caller that built the struct with
/// a literal. Construct it via [`PsOptions::new`] or the `with_*` builders
/// below; a struct literal is refused outside this crate, which is what buys
/// the room to grow.
#[derive(Default)]
#[non_exhaustive]
pub struct PsOptions {
	/// Include stopped containers, `-a/--all` (default: running only).
	pub all: bool,
	/// Print only container IDs, `-q/--quiet`.
	pub quiet: bool,
	/// Emit JSON instead of the table, `--format json`.
	pub json: bool,
	/// Emit one Compose-compatible JSON object per line, `--format ndjson`.
	pub ndjson: bool,
}

impl PsOptions {
	/// Every `docker compose ps` flag, in CLI order. A constructor rather than
	/// a struct literal because the type is `#[non_exhaustive]`, so the next
	/// flag to land is not a breaking change for anyone building one.
	pub fn new(all: bool, quiet: bool, json: bool) -> Self {
		Self {
			all,
			quiet,
			json,
			ndjson: false,
		}
	}

	/// Emit NDJSON instead of the table. JSON and NDJSON builders select one format.
	#[must_use]
	pub fn with_ndjson(mut self, ndjson: bool) -> Self {
		self.ndjson = ndjson;
		if ndjson {
			self.json = false;
		}
		self
	}

	/// Include stopped containers, `-a/--all` (default: running only).
	/// Builder-style.
	#[must_use]
	pub fn with_all(mut self, all: bool) -> Self {
		self.all = all;
		self
	}

	/// Print only container IDs, `-q/--quiet`. Builder-style.
	#[must_use]
	pub fn with_quiet(mut self, quiet: bool) -> Self {
		self.quiet = quiet;
		self
	}

	/// Emit JSON instead of the table, `--format json`. Builder-style.
	#[must_use]
	pub fn with_json(mut self, json: bool) -> Self {
		self.json = json;
		if json {
			self.ndjson = false;
		}
		self
	}
}

/// Options for `ps` added after the crate's API froze.
///
/// **`#[non_exhaustive]` from birth, and that is the whole point.** Both
/// [`PsOptions`] and [`PsFilterOptions`] are externally constructible with a
/// struct literal, so adding a field to either requires a MAJOR, measured with
/// `cargo semver-checks`, which reports `constructible_struct_adds_field`, not
/// assumed from the rules. `PsFilterOptions` was itself introduced to keep
/// `PsOptions` stable and inherited the same problem, so a third frozen struct
/// would only move the wall.
///
/// Construct it with [`Default::default`] and the builder below; a struct
/// literal is refused outside this crate, which is what buys the room to grow.
#[derive(Default, Clone, Copy, Debug)]
#[non_exhaustive]
pub struct PsDisplayOptions {
	/// Ask the server for each container's on-disk size and show the SIZE
	/// column, `-s/--size`.
	///
	/// Off by default because it is not free: libpod walks each container's
	/// writable layer to answer, measured at 21 ms → 109 ms over 59 containers
	/// on Podman 5.7.0. `docker ps -s` is opt-in for the same reason.
	pub size: bool,
}

impl PsDisplayOptions {
	/// Ask for the SIZE column, `-s/--size`. Builder-style.
	#[must_use]
	pub fn with_size(mut self, size: bool) -> Self {
		self.size = size;
		self
	}

	/// The single `ps` display flag, in CLI order. A constructor rather than
	/// a struct literal because the type is `#[non_exhaustive]`, so the next
	/// flag to land is not a breaking change for anyone building one.
	pub fn new(size: bool) -> Self {
		Self { size }
	}
}

/// Service/status/name filters for [`Engine::ps_filtered`](crate::Engine::ps_filtered) (`docker compose ps`
/// `--services`, `[SERVICE...]`, `--status`, `--filter`).
///
/// `#[non_exhaustive]` since 4.0.0, same rationale as [`PsOptions`]: a new
/// filter kind can be added in a minor release without breaking external
/// callers. Construct it via [`PsFilterOptions::new`] or the `with_*` builders
/// below; a struct literal is refused outside this crate.
#[derive(Default)]
#[non_exhaustive]
pub struct PsFilterOptions {
	/// Print the service names instead of the container table, `--services`.
	pub services_only: bool,
	/// Print configured services in file order, `--all-services`.
	pub all_services: bool,
	/// Restrict to these services' containers (positional `SERVICE` filter).
	pub services: Vec<String>,
	/// Status filters, `--status` (e.g. running, exited); OR-combined.
	pub status: Vec<String>,
	/// Generic `KEY=VALUE` predicates, `--filter` (supports status= and name=).
	pub filters: Vec<String>,
}

impl PsFilterOptions {
	/// Every `ps` filter flag, in CLI order. A constructor rather than a struct
	/// literal because the type is `#[non_exhaustive]`, so the next flag to
	/// land is not a breaking change for anyone building one.
	pub fn new(
		services_only: bool,
		services: Vec<String>,
		status: Vec<String>,
		filters: Vec<String>,
	) -> Self {
		Self {
			services_only,
			all_services: false,
			services,
			status,
			filters,
		}
	}

	/// Print configured services, including services without containers. Builder-style.
	#[must_use]
	pub fn with_all_services(mut self, all_services: bool) -> Self {
		self.all_services = all_services;
		self
	}

	/// Generic `KEY=VALUE` predicates, `--filter` (supports status= and name=).
	/// Builder-style.
	#[must_use]
	pub fn with_filters(mut self, filters: Vec<String>) -> Self {
		self.filters = filters;
		self
	}
}
