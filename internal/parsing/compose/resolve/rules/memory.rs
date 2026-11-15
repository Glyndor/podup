//! Memory amounts that cannot be used are an error, not a dropped limit.
//!
//! The engine turns each of these into a byte count with
//! [`crate::size::parse_memory`]; a value that does not parse used to vanish
//! on the way (no limit applied, nothing said). Reading the file now fails
//! instead, naming the service and the key.

use crate::compose::types::Service;
use crate::error::{ComposeError, Result};

/// Reject a memory amount the engine could not turn into a byte count: plain
/// bytes or a `b`/`k`/`m`/`g`/`t` amount (with an optional `b`), and `-1`.
/// An empty value is not a mistake: `${MEM:-}` is how a file says "no limit",
/// so it counts as unset.
pub(in crate::parsing::compose::resolve) fn validate_memory_amounts(
	name: &str,
	service: &Service,
) -> Result<()> {
	let resources = service.deploy.as_ref().and_then(|d| d.resources.as_ref());
	let amounts = [
		("mem_limit", service.mem_limit.as_deref()),
		("memswap_limit", service.memswap_limit.as_deref()),
		("mem_reservation", service.mem_reservation.as_deref()),
		("shm_size", service.shm_size.as_deref()),
		(
			"deploy.resources.limits.memory",
			resources
				.and_then(|r| r.limits.as_ref())
				.and_then(|l| l.memory.as_deref()),
		),
		(
			"deploy.resources.reservations.memory",
			resources
				.and_then(|r| r.reservations.as_ref())
				.and_then(|l| l.memory.as_deref()),
		),
	];
	for (key, value) in amounts {
		if let Some(text) = value
			.filter(|text| !text.trim().is_empty() && crate::size::parse_memory(text).is_none())
		{
			return Err(ComposeError::Invalid(format!(
				"service '{}': {key} {text:?} is not a valid memory amount",
				crate::error::sanitize_name(name)
			)));
		}
	}
	Ok(())
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
