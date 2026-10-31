//! The small converters the fuzz harness reaches, behind `test-helpers`.

/// The soft and hard limit a compose ulimit becomes, for `nofile`.
pub(crate) fn ulimit_pair(soft: i64, hard: i64) -> (u64, u64) {
	super::workload::container_config::resources::ulimit_pair(soft, hard, "nofile")
}

/// What the short-form volume split made of `entry`.
pub(crate) fn classify_short_volume(entry: &str) -> Option<(bool, String, String)> {
	super::objects::volume_mounts::fuzz_hook::classify_short(entry)
}
