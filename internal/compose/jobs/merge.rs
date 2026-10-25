//! Combining a job declared in more than one `-f` file.

use std::collections::HashMap;

use crate::compose::extends::merge_service_tagged;
use crate::compose::tags::MergeTag;
use crate::compose::types::{Job, Triggers};

/// Merge `over` onto `base`, `over` winning, the way a service is merged.
///
/// The container and workload keys follow the service rules, `!override` and
/// `!reset` included. Triggers combine by trigger type: `manual` takes the
/// later file's value when it states one, and a non-empty `schedule` list in
/// the later file replaces the earlier list as a whole. Appending would make
/// an override file unable to remove a schedule, and a merged cron list
/// nobody wrote is the surprise a scheduler must not have.
pub(crate) fn merge_job(base: Job, over: Job, tagged: Option<&HashMap<String, MergeTag>>) -> Job {
	let triggers = Triggers {
		manual: over.triggers.manual.or(base.triggers.manual),
		schedule: if over.triggers.schedule.is_empty() {
			base.triggers.schedule
		} else {
			over.triggers.schedule
		},
	};
	Job {
		service: merge_service_tagged(base.service, over.service, tagged),
		triggers,
	}
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
