use crate::libpod::types::volume::VolumeDiskUsage;

/// The `Usage` object of one volume in `volumes --size --format json`.
///
/// A volume the host does not list (a compose file can declare one that was
/// never created) has `null` counts: the table leaves those cells empty
/// because `0B` would claim the volume exists and is empty, and the JSON says
/// the same. The `Usage` of a run that did not ask for sizes stays `null` as a
/// whole, so the two cases remain apart.
pub(super) fn usage_json(usage: Option<&VolumeDiskUsage>) -> serde_json::Value {
	serde_json::json!({
		"Size": usage.map(|u| u.size),
		"ReclaimableSize": usage.map(|u| u.reclaimable),
		"Links": usage.map(|u| u.links),
	})
}

#[cfg(test)]
#[path = "usage_json_tests.rs"]
mod tests;
