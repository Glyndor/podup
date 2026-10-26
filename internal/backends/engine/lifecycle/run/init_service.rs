//! The service a one-off `run` hands to its init steps.

use crate::compose::types::Service;

/// The service the init steps inherit from a `run`.
///
/// A `run` carries adjustments that describe the throwaway container only:
/// the command it was asked to run, an overridden entrypoint, a TTY, an
/// ad-hoc mount and an ad-hoc label. Those belong to the container the run
/// starts, not to the init work that precedes it, which is a separate
/// container with its own command. What does belong to the service
/// environment is carried over: the user, the working directory and the
/// environment the run assembled.
///
/// The init steps publish no ports: the service's own ports are dropped
/// when a step is completed, and the run's `-p` publishes never reach them.
pub(super) fn init_service(service: &Service, run_service: &Service) -> Service {
	let mut init = service.clone();
	init.user = run_service.user.clone();
	init.working_dir = run_service.working_dir.clone();
	init.environment = run_service.environment.clone();
	init
}
