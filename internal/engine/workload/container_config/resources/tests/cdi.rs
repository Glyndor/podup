use super::*;

// --- cdi devices ---

fn cdi_for(yaml: &str) -> Vec<String> {
	let file = crate::parse_str(yaml).unwrap();
	cdi_devices(&file.services["app"])
}

#[test]
fn cdi_gpu_count_all() {
	let got = cdi_for(
		"services:\n  app:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              count: all\n",
	);
	assert_eq!(got, vec!["nvidia.com/gpu=all"]);
}

#[test]
fn cdi_gpu_count_n_enumerates() {
	let got = cdi_for(
		"services:\n  app:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              count: 2\n",
	);
	assert_eq!(got, vec!["nvidia.com/gpu=0", "nvidia.com/gpu=1"]);
}

#[test]
fn cdi_gpu_device_ids() {
	let got = cdi_for(
		"services:\n  app:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              device_ids: [\"GPU-abc\", \"1\"]\n",
	);
	assert_eq!(got, vec!["nvidia.com/gpu=GPU-abc", "nvidia.com/gpu=1"]);
}

#[test]
fn cdi_top_level_gpus_all() {
	assert_eq!(
		cdi_for("services:\n  app:\n    image: x\n    gpus: all\n"),
		vec!["nvidia.com/gpu=all"]
	);
}

#[test]
fn cdi_top_level_gpus_count() {
	assert_eq!(
		cdi_for("services:\n  app:\n    image: x\n    gpus: 2\n"),
		vec!["nvidia.com/gpu=0", "nvidia.com/gpu=1"]
	);
}

#[test]
fn cdi_top_level_gpus_device_list() {
	assert_eq!(
		cdi_for(
			"services:\n  app:\n    image: x\n    gpus:\n      - capabilities: [gpu]\n        device_ids: [\"GPU-xyz\"]\n",
		),
		vec!["nvidia.com/gpu=GPU-xyz"]
	);
}

#[test]
fn cdi_non_gpu_skipped() {
	let got = cdi_for(
		"services:\n  app:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [tpu]\n              driver: google\n",
	);
	assert!(got.is_empty());
}

#[test]
fn cdi_absent_without_deploy() {
	assert!(cdi_devices(&default_service()).is_empty());
}

// --- gpu count clamp ---

#[test]
fn cdi_gpu_count_is_clamped() {
	let yaml = format!(
		"services:\n  g:\n    image: x\n    deploy:\n      resources:\n        reservations:\n          devices:\n            - capabilities: [gpu]\n              count: {}\n",
		MAX_GPU_DEVICES + 10_000
	);
	let file = crate::compose::parse_str(&yaml).unwrap();
	let out = cdi_devices(&file.services["g"]);
	assert_eq!(out.len(), MAX_GPU_DEVICES as usize);
}
