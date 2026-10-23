use super::*;

// --- engine-side pinning (`build_resource_limits` forward contract) ---

/// `cpu_quota: -1` with `cpus: "2"` must forward `quota: -1` into
/// `LinuxCPU.quota` exactly the way it did before the audit helpers
/// landed. A regression that re-introduces a filter on `cpu_quota`
/// would silently flip this to `200000` and disagree with the
/// pre-branch engine output (`#1894`).
#[test]
fn build_resource_limits_forwards_negative_one_cpu_quota_with_cpus() {
	let mut svc = default_service();
	svc.cpu_quota = Some(-1);
	svc.cpus = Some("2".into());
	let res = build_resource_limits(&svc).unwrap();
	let cpu = res.cpu.unwrap();
	assert_eq!(
		cpu.quota,
		Some(-1),
		"`cpu_quota: -1` must win over `cpus: \"2\"` and be forwarded as -1"
	);
	// The default 100ms period is added because the quota is in
	// effect, matching the pre-branch behavior.
	assert_eq!(cpu.period, Some(100_000));
}

/// `mem_limit: "-1"` must be forwarded as `Some(-1)` into
/// `LinuxMemory.limit`, exactly the way the pre-branch code did. A
/// regression that re-introduces a filter on the helper would
/// silently drop this to `None` and disagree with the pre-branch
/// engine output (`#1894`).
#[test]
fn build_resource_limits_forwards_negative_one_mem_limit() {
	let mut svc = default_service();
	svc.mem_limit = Some("-1".into());
	let res = build_resource_limits(&svc).unwrap();
	let mem = res.memory.unwrap();
	assert_eq!(
		mem.limit,
		Some(-1),
		"`mem_limit: \"-1\"` must be forwarded as -1, not dropped"
	);
}
