//! Rule selection for a changed path.
//!
//! A path can be watched by more than one rule: two services may sync the
//! same directory into their own targets, and one service may declare two
//! rules over the same path. Every rule that watches the path has to run,
//! so the selection returns all of them; the first match used to end the
//! scan, and the second service never saw the change.
//!
//! The whole-container actions (`rebuild`, `restart`) collapse per batch:
//! their effect is the container rather than the file, so repeating them
//! for the next changed path of the same batch is pure waste. The collapse
//! is per service (`rebuild`) and per container (`restart`), and it removes
//! only the rule that would repeat the action: the other rules watching the
//! same path still run.

use std::collections::HashSet;
use std::path::Path;

use crate::compose::types::WatchAction;

use super::RuleEntry;

/// The identity of a whole-container action that already ran in this batch:
/// the kind of action, and the service or container it applies to.
pub(super) type DoneKey = (u8, String);

/// The rules that watch `path`, in declaration order.
///
/// `accepts` decides whether a rule's own ignore/include filters let the
/// path through; it runs only for the rules whose root contains the path.
/// `done` carries the whole-container actions that already ran in this
/// batch: a rule whose action already ran is left out, and the rules after
/// it are still returned.
pub(super) fn matching_rules<'a, F>(
	path: &Path,
	rule_entries: &'a [RuleEntry],
	done: &mut HashSet<DoneKey>,
	mut accepts: F,
) -> Vec<&'a RuleEntry>
where
	F: FnMut(&RuleEntry) -> bool,
{
	let mut out = Vec::new();
	for entry in rule_entries {
		if !path.starts_with(&entry.abs_path) {
			continue;
		}
		if !accepts(entry) {
			continue;
		}
		// Collapse repeated rebuild/restart of the same container within
		// this batch into one; the action's effect is whole-container, so
		// a second run is pure waste.
		let dedup_key = match &entry.rule.action {
			WatchAction::Rebuild => Some((0, entry.service_name.clone())),
			WatchAction::Restart => Some((1, entry.container_name.clone())),
			_ => None,
		};
		if let Some(key) = dedup_key {
			if !done.insert(key) {
				continue;
			}
		}
		out.push(entry);
	}
	out
}

#[cfg(test)]
#[path = "matching_tests.rs"]
mod tests;
