use super::super::filters::{select_containers, ServiceFilter};
use super::super::services::{configured_services, container_services};
use super::*;

fn service(name: &str, state: &str) -> ContainerListEntry {
	let mut c = entry("", state);
	c.labels.insert("podup.service".into(), name.into());
	c
}

#[test]
fn services_are_distinct_sorted_and_only_from_selected_containers() {
	let containers = vec![
		service("web", "running"),
		service("db", "exited"),
		service("api", "running"),
		service("web", "running"),
	];
	assert_eq!(
		container_services(&containers)
			.into_iter()
			.collect::<Vec<_>>(),
		["api", "db", "web"]
	);
	let selected = select_containers(containers, None, &["running".into()], &[]);
	assert_eq!(
		container_services(&selected)
			.into_iter()
			.collect::<Vec<_>>(),
		["api", "web"]
	);
	let filter = ServiceFilter::new(["web".into()].into(), Default::default());
	let selected = select_containers(selected, Some(&filter), &["running".into()], &[]);
	assert_eq!(
		container_services(&selected)
			.into_iter()
			.collect::<Vec<_>>(),
		["web"]
	);
}

#[test]
fn services_honour_name_filter_and_use_the_table_fallback() {
	let mut api = service("api", "running");
	api.names = vec!["/api-1".into()];
	let selected = select_containers(
		vec![api, service("web", "running")],
		None,
		&[],
		&["api-".into()],
	);
	assert_eq!(
		container_services(&selected)
			.into_iter()
			.collect::<Vec<_>>(),
		["api"]
	);
	assert!(container_services(&[]).is_empty());
}

#[test]
fn a_container_without_a_service_label_is_not_listed_as_an_empty_service() {
	let unlabelled = entry("", "running");
	assert_eq!(table_service(&unlabelled), "");
	assert!(container_services(std::slice::from_ref(&unlabelled)).is_empty());
	let both = [unlabelled, service("web", "running")];
	assert_eq!(
		container_services(&both).into_iter().collect::<Vec<_>>(),
		["web"]
	);
}

#[test]
fn a_service_label_cannot_drive_the_terminal_or_split_the_list() {
	// An escape sequence and a newline in a label from the daemon: one line
	// per service has to stay true, and nothing may reach the terminal raw.
	let hostile = service("web\x1b[2J\nfake", "running");
	let listed = container_services(std::slice::from_ref(&hostile));
	assert_eq!(listed.len(), 1);
	let line = listed.into_iter().next().unwrap();
	assert!(!line.chars().any(char::is_control), "{line:?}");
	assert!(!line.contains('\n'), "{line:?}");
	assert!(line.starts_with("web"), "{line:?}");
}

#[test]
fn all_services_preserves_file_order_and_positional_selection() {
	let file = crate::compose::parse_str(
		"services:\n  zebra: {image: alpine}\n  api: {image: alpine}\n  web: {image: alpine}\n",
	)
	.unwrap();
	assert_eq!(configured_services(&file, &[]), ["zebra", "api", "web"]);
	assert_eq!(
		configured_services(&file, &["web".into(), "zebra".into()]),
		["zebra", "web"]
	);
	assert!(configured_services(&ComposeFile::default(), &[]).is_empty());
}
