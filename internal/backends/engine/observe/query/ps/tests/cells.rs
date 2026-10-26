use super::*;

/// CREATED is how long ago the container was made, parsed from the RFC 3339
/// string libpod sends in `Created`.
#[test]
fn the_created_cell_reports_the_age_of_the_container() {
	let c = ContainerListEntry {
		image_id: String::new(),
		// 2026-08-02 22:34:41 at -05:00 is 2026-08-03 03:34:41Z, which is two
		// hours and change after NOW... so use a value comfortably before it.
		created: "2026-08-01T00:58:45Z".into(),
		..entry("", "running")
	};
	assert_eq!(table_created(&c, NOW), "2 days ago");
}

/// A timestamp podup cannot parse leaves the cell blank rather than showing a
/// plausible wrong age. The wrong age is the one a reader would act on.
#[test]
fn an_unparseable_created_leaves_the_cell_blank() {
	for bad in ["", "not a timestamp", "2026-13-01T00:00:00Z"] {
		let c = ContainerListEntry {
			image_id: String::new(),
			created: bad.into(),
			..entry("", "running")
		};
		assert_eq!(table_created(&c, NOW), "", "{bad:?}");
	}
}

/// The SERVICE cell carries the row's `podup.service` label verbatim. Fourteen
/// subcommands (`logs`, `exec`, `top`, `pause`, `unpause`, `stop`, `start`,
/// `restart`, `kill`, `rm`, `wait`, `attach`, `commit`, `export`) accept the
/// service name and reject the container name, so the value printed here is
/// what a reader needs to copy to drive them.
#[test]
fn the_service_cell_carries_the_podup_service_label() {
	let mut labels = HashMap::new();
	labels.insert("podup.service".to_string(), "web".to_string());
	let c = ContainerListEntry {
		image_id: String::new(),
		labels,
		..entry("", "running")
	};
	assert_eq!(table_service(&c), "web");
}

/// A container with no `podup.service` label renders an empty cell, the same
/// way other cells say "unknown" (CREATED with an unparseable timestamp, SIZE
/// without `-s`). A blank here means podup could not tell; "1" or "?" would be
/// a guess.
#[test]
fn the_service_cell_is_blank_when_the_label_is_missing() {
	assert_eq!(table_service(&entry("", "running")), "");
}
