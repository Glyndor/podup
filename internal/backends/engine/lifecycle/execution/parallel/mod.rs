//! Intra-level parallelism for the lifecycle commands.
//!
//! The order resolver groups services into dependency *levels*
//! ([`crate::compose::resolve_levels`]): every service in one level has its
//! `depends_on` satisfied by an earlier level, so services *within* a level have
//! no ordering between them and can be acted on concurrently. The whole-project
//! lifecycle commands (stop/start/restart/kill/rm/pause/unpause/down) walk the
//! levels in order, preserving the cross-level dependency ordering, but
//! dispatch each level's per-service (or, for teardown, per-container)
//! operations in parallel instead of strictly serially, so a restart/stop/down
//! of many independent services no longer serializes every grace period (#757).
//! This mirrors what the `up`/`create` path already does.

use std::collections::HashSet;
use std::sync::Arc;

use crate::compose::dependencies::effective_depends_on;
use crate::compose::types::ComposeFile;
use crate::error::{ComposeError, Result};

/// Upper bound on the number of same-level services a lifecycle command acts on
/// concurrently. Services within a dependency level have no ordering between
/// them, so they run in parallel; the cap keeps a very wide compose file from
/// opening an unbounded number of simultaneous libpod connections at once.
pub(in crate::backends::engine) const MAX_LIFECYCLE_CONCURRENCY: usize = 16;

/// Run a batch of independent per-service futures concurrently, bounded by
/// [`MAX_LIFECYCLE_CONCURRENCY`], and return their outputs in the *input*
/// order (not completion order) so error selection and reporting stay
/// deterministic regardless of which service happens to finish first.
/// Generic over the output type so both a fallible per-service unit
/// (`Result<()>`, reduced via [`first_error`]) and a best-effort one that
/// never fails (`()`, e.g. the image-prefetch stage) share this one bounded
/// dispatcher.
///
/// Not reachable from every stage that wants a fan-out: this is built on
/// `buffer_unordered`, whose `FuturesUnordered` is `Send` only if its future is
/// `Send` for *every* lifetime. A caller whose futures borrow `&self` therefore
/// acquires a higher-ranked bound that propagates out to its own callers, which
/// is why the secret pre-creation stage (#1219) chunks `join_all` against
/// [`MAX_LIFECYCLE_CONCURRENCY`] instead of calling this. The cap is shared; the
/// dispatcher is not.
pub(in crate::backends::engine::lifecycle) async fn join_bounded<F, T>(
	futs: impl IntoIterator<Item = F>,
) -> Vec<T>
where
	F: std::future::Future<Output = T>,
	T: Send,
{
	use futures_util::stream::StreamExt;
	let mut indexed: Vec<(usize, T)> = futures_util::stream::iter(
		futs.into_iter()
			.enumerate()
			.map(|(i, fut)| async move { (i, fut.await) }),
	)
	.buffer_unordered(MAX_LIFECYCLE_CONCURRENCY)
	.collect()
	.await;
	indexed.sort_by_key(|(i, _)| *i);
	indexed.into_iter().map(|(_, r)| r).collect()
}

/// Reduce a level's per-service results to the first error in service order, so
/// one failing service is still reported clearly while the rest of the level is
/// allowed to complete.
pub(in crate::backends::engine) fn first_error(results: Vec<Result<()>>) -> Option<ComposeError> {
	results.into_iter().find_map(Result::err)
}

/// Filter each dependency level down to `target_services`, dropping levels that
/// end up empty. An empty target list keeps every level. Returns an error if any
/// requested name is not in the file, matching [`crate::engine::lifecycle::targets::filter_services`]
/// (and docker compose's "no such service").
pub(in crate::backends::engine::lifecycle) fn filter_levels(
	file: &ComposeFile,
	levels: Vec<Vec<String>>,
	target_services: &[String],
) -> Result<Vec<Vec<String>>> {
	for name in target_services {
		if !file.services.contains_key(name) {
			return Err(ComposeError::ServiceNotFound(name.clone()));
		}
	}
	if target_services.is_empty() {
		return Ok(levels);
	}
	let set: HashSet<&str> = target_services.iter().map(|s| s.as_str()).collect();
	Ok(retain_levels(levels, |n| set.contains(n)))
}

/// Keep only the level entries matching `keep`, dropping levels left empty.
pub(in crate::backends::engine::lifecycle) fn retain_levels(
	levels: Vec<Vec<String>>,
	keep: impl Fn(&str) -> bool,
) -> Vec<Vec<String>> {
	levels
		.into_iter()
		.map(|level| {
			level
				.into_iter()
				.filter(|n| keep(n.as_str()))
				.collect::<Vec<_>>()
		})
		.filter(|level| !level.is_empty())
		.collect()
}

/// Compute the set of services a `restart` should act on, plus the explicit
/// target subset (used only to label cascade-restarts distinctly in the logs).
///
/// With no targets every service is restarted. With targets, the set is the
/// targets plus, unless `no_deps`, every service whose `depends_on` carries a
/// `restart: true` condition pointing at one of the targets (one hop, matching
/// the previous serial implementation).
pub(in crate::backends::engine::lifecycle) fn restart_service_set(
	file: &ComposeFile,
	target_services: &[String],
	no_deps: bool,
) -> (Arc<HashSet<String>>, Arc<HashSet<String>>) {
	if target_services.is_empty() {
		// No targets means "every service is both a target and a member of
		// the full restart set". The two `Arc`s share the same `HashSet` so
		// only one `HashSet` is built and only one `Arc::clone` is paid;
		// callers read both as immutable (`contains()`), so the aliasing is
		// safe. The previous code wrote `(all.clone(), all)` inline (#1364).
		let all: Arc<HashSet<String>> = Arc::new(file.services.keys().cloned().collect());
		(all.clone(), all)
	} else {
		let targets: Arc<HashSet<String>> = Arc::new(target_services.iter().cloned().collect());
		let mut full: HashSet<String> = targets.as_ref().clone();
		if !no_deps {
			for (dep_name, dep_service) in &file.services {
				let deps = effective_depends_on(dep_service, &file.services);
				if targets.iter().any(|t| deps.restart_for(t)) {
					full.insert(dep_name.clone());
				}
			}
		}
		(Arc::new(full), targets)
	}
}

mod service_ops;
mod teardown;

#[cfg(test)]
mod tests;
