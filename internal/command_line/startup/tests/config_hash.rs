use super::*;
use crate::cli::ConfigFormat;
use std::path::Path;

fn hash(selector: &str) -> (podup::Result<()>, String) {
	let file =
		podup::parse_str("services:\n  web:\n    image: nginx\n  db:\n    image: pg\n").unwrap();
	let mut out = Vec::new();
	let result = render_config_to(
		&mut out,
		&file,
		&ConfigFormat::Yaml,
		&ConfigOutput {
			hash: Some(selector.to_string()),
			..Default::default()
		},
		"proj",
		Path::new("/proj"),
	);
	(result, String::from_utf8(out).unwrap())
}

#[test]
fn an_unknown_name_leaves_nothing_on_stdout() {
	let (result, written) = hash("web,bogus");
	assert!(matches!(result, Err(podup::ComposeError::ServiceNotFound(name)) if name == "bogus"));
	assert_eq!(written, "", "a partial answer was printed before the error");
}

#[test]
fn known_names_print_one_line_each_in_the_order_given() {
	let (result, written) = hash("db,web");
	result.unwrap();
	let names: Vec<&str> = written
		.lines()
		.map(|l| l.split(' ').next().unwrap())
		.collect();
	assert_eq!(names, ["db", "web"]);
}
