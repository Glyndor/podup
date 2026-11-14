use super::*;
use crate::libpod::types::container::LinuxBlockIO;

fn host(doc: serde_json::Value) -> HostCgroups {
	serde_json::from_value::<Info>(doc).unwrap().cgroups()
}

fn v2(controllers: &[&str]) -> HostCgroups {
	host(serde_json::json!({"host": {"cgroupVersion": "v2", "cgroupControllers": controllers}}))
}

fn v1(controllers: &[&str]) -> HostCgroups {
	host(serde_json::json!({"host": {"cgroupVersion": "v1", "cgroupControllers": controllers}}))
}

/// Every value whose fate depends on the host, next to a memory limit that
/// never does.
fn every_host_dependent_limit() -> LinuxResources {
	LinuxResources {
		memory: Some(LinuxMemory {
			limit: Some(512 << 20),
			swappiness: Some(10),
			disable_oom_killer: Some(true),
			..LinuxMemory::default()
		}),
		cpu: Some(LinuxCPU {
			realtime_period: Some(1_000_000),
			realtime_runtime: Some(950_000),
			..LinuxCPU::default()
		}),
		block_io: Some(LinuxBlockIO {
			weight: Some(300),
			..LinuxBlockIO::default()
		}),
		..LinuxResources::default()
	}
}

#[test]
fn cgroups_v2_drops_the_v1_only_knobs_and_keeps_the_rest() {
	let mut limits = every_host_dependent_limit();
	let messages = unsupported_limits(&mut limits, Some(&v2(&["cpu", "io", "memory", "pids"])));
	let memory = limits.memory.as_ref().expect("the memory limit stays");
	assert_eq!(memory.limit, Some(512 << 20));
	assert!(memory.swappiness.is_none() && memory.disable_oom_killer.is_none());
	// Nothing else was in the CPU block, so it goes as a whole.
	assert!(limits.cpu.is_none());
	// `io` is delegated, so the block-I/O limits stay.
	assert!(limits.block_io.is_some());
	let joined = messages.join("\n");
	for field in [
		"oom_kill_disable",
		"mem_swappiness",
		"cpu_rt_period",
		"cpu_rt_runtime",
	] {
		assert!(
			joined.contains(&format!("{field} is discarded")),
			"{field}: {joined}"
		);
	}
	assert!(!joined.contains("blkio_config"), "{joined}");
}

#[test]
fn a_host_without_the_io_controller_drops_the_block_io_limits() {
	let mut limits = every_host_dependent_limit();
	let messages = unsupported_limits(&mut limits, Some(&v2(&["cpu", "memory", "pids"])));
	assert!(limits.block_io.is_none());
	assert!(
		messages
			.iter()
			.any(|m| m.contains("blkio_config is discarded") && m.contains("\"io\"")),
		"{messages:?}"
	);
}

#[test]
fn cgroups_v1_keeps_everything_and_says_nothing() {
	let mut limits = every_host_dependent_limit();
	let messages = unsupported_limits(&mut limits, Some(&v1(&["blkio", "cpu", "memory", "pids"])));
	assert!(messages.is_empty(), "{messages:?}");
	let memory = limits.memory.as_ref().unwrap();
	assert_eq!(memory.swappiness, Some(10));
	assert_eq!(memory.disable_oom_killer, Some(true));
	assert_eq!(limits.cpu.as_ref().unwrap().realtime_runtime, Some(950_000));
	assert!(limits.block_io.is_some());
}

#[test]
fn cgroups_v1_names_its_block_io_controller_blkio() {
	// v1 never lists `io`; reading it as the v2 name would drop limits a v1
	// host applies.
	let mut limits = every_host_dependent_limit();
	unsupported_limits(&mut limits, Some(&v1(&["blkio"])));
	assert!(limits.block_io.is_some());
	let mut limits = every_host_dependent_limit();
	let messages = unsupported_limits(&mut limits, Some(&v1(&["cpu", "memory"])));
	assert!(limits.block_io.is_none());
	assert!(
		messages.iter().any(|m| m.contains("\"blkio\"")),
		"{messages:?}"
	);
}

#[test]
fn an_unknown_host_keeps_every_value_and_reports_it_unchecked() {
	let mut limits = every_host_dependent_limit();
	let messages = unsupported_limits(&mut limits, None);
	assert_eq!(limits.memory.as_ref().unwrap().swappiness, Some(10));
	assert!(limits.block_io.is_some());
	assert_eq!(messages.len(), 5, "{messages:?}");
	assert!(
		messages.iter().all(|m| m.contains("is sent as declared")),
		"{messages:?}"
	);
}

#[test]
fn a_memory_block_left_empty_is_not_sent() {
	let mut limits = LinuxResources {
		memory: Some(LinuxMemory {
			swappiness: Some(10),
			..LinuxMemory::default()
		}),
		..LinuxResources::default()
	};
	unsupported_limits(&mut limits, Some(&v2(&["memory"])));
	assert!(limits.memory.is_none());
}

/// The create body of an `up` against a fake daemon whose `/info` answers
/// `info`, and how many times `/info` was asked.
#[cfg(unix)]
async fn up_against(info: &'static str, services: &str) -> (Vec<serde_json::Value>, usize) {
	let fake = crate::engine::fake_podman::start(move |method, target| {
		if method == "GET" && target.ends_with("/info") {
			(200, info.to_string())
		} else if method == "GET" && target.contains("/containers/json") {
			(200, "[]".to_string())
		} else if method == "POST" && target.contains("/images/pull") {
			(200, String::new())
		} else if method == "POST" && target.contains("/containers/create") {
			(200, "{}".to_string())
		} else if method == "POST" && target.contains("/start") {
			(200, String::new())
		} else {
			(404, r#"{"message":"not found"}"#.to_string())
		}
	});
	let e = Engine::with_base_dir(fake.client(), "proj".into(), std::env::temp_dir());
	let file = crate::parse_str(&format!("services:\n{services}")).unwrap();
	e.up_with_options(&file, false, &[], &[], false, false, false, false)
		.await
		.expect("up must succeed");
	let reqs = fake.requests.lock().unwrap();
	let bodies = fake.bodies.lock().unwrap();
	let creates = reqs
		.iter()
		.zip(bodies.iter())
		.filter(|(r, _)| r.contains("/containers/create"))
		.map(|(_, b)| serde_json::from_slice(b).unwrap())
		.collect();
	let infos = reqs.iter().filter(|r| r.ends_with("/info")).count();
	(creates, infos)
}

#[cfg(unix)]
const LIMITED: &str = "  web:\n    image: img\n    runtime: runc\n    mem_limit: 512m\n    mem_swappiness: 10\n    oom_kill_disable: true\n    cpu_rt_runtime: 950000\n    cpu_rt_period: 1000000\n    blkio_config:\n      weight: 300\n      weight_device:\n        - path: /dev/null\n          weight: 200\n      device_read_bps:\n        - path: /dev/null\n          rate: 1mb\n";

#[cfg(unix)]
#[tokio::test]
async fn a_cgroups_v1_host_gets_every_limit_under_the_name_podman_reads() {
	let info =
		r#"{"host":{"cgroupVersion":"v1","cgroupControllers":["blkio","cpu","memory","pids"]}}"#;
	let (bodies, _) = up_against(info, LIMITED).await;
	let body = &bodies[0];
	// Podman reads the runtime from `oci_runtime`; `runtime` it ignores.
	assert_eq!(body["oci_runtime"], "runc");
	assert!(body.get("runtime").is_none(), "{body}");
	let limits = &body["resource_limits"];
	assert_eq!(limits["memory"]["disableOOMKiller"], true);
	assert_eq!(limits["memory"]["swappiness"], 10);
	assert_eq!(limits["cpu"]["realtimeRuntime"], 950_000);
	assert_eq!(limits["cpu"]["realtimePeriod"], 1_000_000);
	let block = &limits["blockIO"];
	assert_eq!(block["weight"], 300);
	assert_eq!(block["weightDevice"][0]["weight"], 200);
	assert_eq!(block["throttleReadBpsDevice"][0]["rate"], 1_048_576);
	for snake in [
		"block_io",
		"disable_oom_killer",
		"realtime_runtime",
		"weight_device",
	] {
		assert!(!body.to_string().contains(snake), "{snake} in {body}");
	}
}

#[cfg(unix)]
#[tokio::test]
async fn a_rootless_cgroups_v2_host_gets_only_what_it_can_apply() {
	let info = r#"{"host":{"cgroupVersion":"v2","cgroupControllers":["cpu","memory","pids"],"security":{"rootless":true}}}"#;
	let (bodies, _) = up_against(info, LIMITED).await;
	let limits = &bodies[0]["resource_limits"];
	assert_eq!(limits["memory"]["limit"], 512_i64 << 20);
	for gone in [
		&limits["memory"]["disableOOMKiller"],
		&limits["memory"]["swappiness"],
		&limits["cpu"]["realtimeRuntime"],
		&limits["blockIO"],
	] {
		assert!(gone.is_null(), "{limits}");
	}
}

#[cfg(unix)]
#[tokio::test]
async fn the_daemon_is_asked_about_its_host_once_per_up() {
	let info = r#"{"host":{"cgroupVersion":"v2","cgroupControllers":["cpu","memory","pids"]}}"#;
	let services = "  a:\n    image: img\n    mem_swappiness: 10\n  b:\n    image: img\n    oom_kill_disable: true\n";
	let (bodies, infos) = up_against(info, services).await;
	assert_eq!(bodies.len(), 2);
	assert_eq!(infos, 1);
}
