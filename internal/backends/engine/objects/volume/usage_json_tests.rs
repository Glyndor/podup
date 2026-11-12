use super::usage_json;
use crate::libpod::types::volume::VolumeDiskUsage;

fn usage(size: u64, reclaimable: u64, links: u64) -> VolumeDiskUsage {
	VolumeDiskUsage {
		name: "vol".into(),
		size,
		reclaimable,
		links,
	}
}

#[test]
fn a_volume_on_the_host_reports_its_counts() {
	let u = usage(4096, 4096, 0);
	let json = usage_json(Some(&u));
	assert_eq!(json["Size"], 4096);
	assert_eq!(json["ReclaimableSize"], 4096);
	assert_eq!(json["Links"], 0);
}

/// The table leaves a volume the host does not list empty, because `0B` would
/// claim it exists and is empty. The JSON says the same with nulls, and keeps
/// them apart from the absent `Usage` of a run that did not ask for sizes.
#[test]
fn a_volume_the_host_does_not_list_has_null_counts_not_zero() {
	let json = usage_json(None);
	assert!(json["Size"].is_null(), "got {json}");
	assert!(json["ReclaimableSize"].is_null(), "got {json}");
	assert!(json["Links"].is_null(), "got {json}");
}

#[test]
fn an_existing_empty_volume_reports_zero() {
	let u = usage(0, 0, 0);
	let json = usage_json(Some(&u));
	assert_eq!(json["Size"], 0);
}
