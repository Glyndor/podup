//! The public option structs for `run`, and the per-field builders that
//! construct them.
//!
//! They live apart from the engine methods in `mod.rs` because they are
//! surface, not behaviour: every field here is something an external caller
//! sets, and every `with_*` is part of the published API. Keeping them
//! together makes the whole `run` contract readable in one file.

/// Options for [`crate::Engine::run`].
///
/// `#[non_exhaustive]` since 4.0.0, so a new field can be added in a minor
/// release without breaking every external caller that built the struct with
/// a literal. Construct it via [`RunOptions::new`] or the `with_*` builders
/// below; a struct literal is refused outside this crate, which is what buys
/// the room to grow.
#[derive(Default)]
#[non_exhaustive]
pub struct RunOptions {
	/// Override the default service command.
	pub cmd: Vec<String>,
	/// Remove the container after it exits.
	pub rm: bool,
	/// Start the container in the background without streaming logs.
	pub detach: bool,
	/// Additional environment variables (`KEY=VAL` strings, override service env).
	pub env_overrides: Vec<String>,
	/// Override the generated container name.
	pub name_override: Option<String>,
	/// Publish the service's declared `ports:` (compose `run --service-ports`).
	/// When false, `run` leaves ports unpublished to avoid host-port collisions.
	pub service_ports: bool,
}

impl RunOptions {
	/// Every `docker compose run` option that lives on the published struct,
	/// in CLI order. A constructor rather than a struct literal because the
	/// type is `#[non_exhaustive]`, so the next field to land is not a
	/// breaking change for anyone building one.
	#[allow(clippy::too_many_arguments)]
	pub fn new(
		cmd: Vec<String>,
		rm: bool,
		detach: bool,
		env_overrides: Vec<String>,
		name_override: Option<String>,
		service_ports: bool,
	) -> Self {
		Self {
			cmd,
			rm,
			detach,
			env_overrides,
			name_override,
			service_ports,
		}
	}
}

/// Extra `docker compose run` flag overrides threaded through the engine
/// builder ([`crate::Engine::with_run_overrides`]).
///
/// `#[non_exhaustive]` since 4.0.0, same rationale as [`RunOptions`]: the
/// next flag to land is not a breaking change for anyone building one with a
/// builder or constructor.
#[derive(Default, Clone)]
#[non_exhaustive]
pub struct RunOverrides {
	/// Run the command as this user (`-u/--user`, `name or UID[:GID]`).
	pub user: Option<String>,
	/// Working directory inside the container (`-w/--workdir`).
	pub workdir: Option<String>,
	/// Override the image entrypoint (`--entrypoint`).
	pub entrypoint: Option<String>,
	/// Extra ad-hoc volume mounts in compose short form (`-v/--volume`).
	pub volumes: Vec<String>,
	/// Extra published ports in compose short form (`-p/--publish`).
	pub publish: Vec<String>,
	/// Keep STDIN open on the container (`-i/--interactive`).
	pub interactive: bool,
	/// Do not start `depends_on` services before the run (`--no-deps`).
	pub no_deps: bool,
}

impl RunOverrides {
	/// Every `docker compose run` override, in CLI order. A constructor rather
	/// than a struct literal because the type is `#[non_exhaustive]`, so the
	/// next flag to land is not a breaking change for anyone building one.
	#[allow(clippy::too_many_arguments)]
	pub fn new(
		user: Option<String>,
		workdir: Option<String>,
		entrypoint: Option<String>,
		volumes: Vec<String>,
		publish: Vec<String>,
		interactive: bool,
		no_deps: bool,
	) -> Self {
		Self {
			user,
			workdir,
			entrypoint,
			volumes,
			publish,
			interactive,
			no_deps,
		}
	}

	/// Run the command as this user (`-u/--user`, `name or UID[:GID]`).
	/// Builder-style.
	#[must_use]
	pub fn with_user(mut self, user: Option<String>) -> Self {
		self.user = user;
		self
	}

	/// Working directory inside the container (`-w/--workdir`). Builder-style.
	#[must_use]
	pub fn with_workdir(mut self, workdir: Option<String>) -> Self {
		self.workdir = workdir;
		self
	}

	/// Extra ad-hoc volume mounts in compose short form (`-v/--volume`).
	/// Builder-style.
	#[must_use]
	pub fn with_volumes(mut self, volumes: Vec<String>) -> Self {
		self.volumes = volumes;
		self
	}
}
