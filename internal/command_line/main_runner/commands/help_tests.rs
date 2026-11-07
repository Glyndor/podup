use super::command_help;

fn help(words: &[&str]) -> String {
	let words: Vec<String> = words.iter().map(|w| (*w).to_string()).collect();
	command_help(&words).to_string()
}

fn usage(text: &str) -> &str {
	text.lines()
		.find(|l| l.starts_with("Usage:"))
		.expect("a Usage line")
}

#[test]
fn the_usage_line_is_the_one_podup_up_help_prints() {
	assert_eq!(
		usage(&help(&["up"])),
		"Usage: podup up [OPTIONS] [SERVICES]..."
	);
}

#[test]
fn the_deepest_named_subcommand_is_the_one_shown() {
	assert!(usage(&help(&["generate", "quadlet"])).starts_with("Usage: podup generate quadlet "));
	assert!(usage(&help(&["autostart", "install"])).starts_with("Usage: podup autostart install "));
	assert!(usage(&help(&["generate"])).starts_with("Usage: podup generate "));
}

#[test]
fn a_nested_help_does_not_offer_a_help_subcommand_that_does_not_exist() {
	for words in [&["generate"][..], &["autostart"][..]] {
		let text = help(words);
		assert!(
			!text.lines().any(|l| l.trim_start().starts_with("help ")),
			"{words:?}:\n{text}"
		);
	}
}

#[test]
fn flags_and_unknown_names_fall_back_as_before() {
	assert!(usage(&help(&["--help", "up"])).starts_with("Usage: podup up "));
	assert!(usage(&help(&["nonsense"])).starts_with("Usage: podup [OPTIONS]"));
	assert!(usage(&help(&[])).starts_with("Usage: podup [OPTIONS]"));
	// Past the last known name the deepest one found stays.
	assert!(usage(&help(&["generate", "nonsense"])).starts_with("Usage: podup generate "));
}
