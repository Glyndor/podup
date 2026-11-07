//! `debian/podup.1` must agree with the CLI about which commands and flags exist.
//!
//! `docs_flags.rs` ties the markdown reference to `--help`, and nothing tied
//! the manual page to it, so the page went on describing a CLI that had since
//! moved. This diffs the page against `--help` in both directions: a flag or a
//! command the page names that the CLI does not have, and a command the CLI
//! has that the page leaves out. It checks names, not descriptions.

use std::collections::BTreeSet;
use std::process::Command;

const MAN_PAGE: &str = include_str!("../../../debian/podup.1");

/// The page as the binary under test can honour it. `update` is a Cargo
/// feature that package-manager builds leave off (`debian/rules` builds
/// `--no-default-features --features watch,completions`), so that build has no
/// `update` command and no `--check`, and the page still documents both. The
/// section of the page that belongs to `update` is left out when the feature is
/// off, which keeps the comparison honest for both builds.
fn page() -> String {
	if cfg!(feature = "update") {
		return MAN_PAGE.to_string();
	}
	let mut out = String::new();
	let mut skipping = false;
	for line in MAN_PAGE.lines() {
		if line == ".B update" || line.starts_with(".B update ") {
			skipping = true;
		} else if skipping && (line.starts_with(".B ") || line.starts_with(".SH ")) {
			skipping = false;
		}
		if !skipping {
			out.push_str(line);
			out.push('\n');
		}
	}
	out
}

/// Flags `--help` never prints: clap's own `--version`, and `events --json`, a
/// hidden alias the page mentions as deprecated.
const NOT_IN_HELP: [&str; 2] = ["--version", "--json"];

fn help(args: &[&str]) -> String {
	let out = Command::new(env!("CARGO_BIN_EXE_podup"))
		.args(args)
		.arg("--help")
		.output()
		.unwrap_or_else(|e| panic!("run {args:?} --help: {e}"));
	assert!(out.status.success(), "`{args:?} --help` failed");
	String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The first word of each line under `Commands:`, `help` left out.
fn subcommands(help: &str) -> Vec<String> {
	help.lines()
		.skip_while(|line| line.trim_end() != "Commands:")
		.skip(1)
		.take_while(|line| line.starts_with("  "))
		.filter_map(|line| line.split_whitespace().next())
		.filter(|name| *name != "help")
		.map(str::to_string)
		.collect()
}

/// Every long flag in `text`: `--name` where the name is lowercase letters,
/// digits and dashes.
fn long_flags(text: &str) -> BTreeSet<String> {
	let mut out = BTreeSet::new();
	for (at, _) in text.match_indices("--") {
		let name: String = text[at + 2..]
			.chars()
			.take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '-')
			.collect();
		let name = name.trim_end_matches('-');
		if name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
			out.insert(format!("--{name}"));
		}
	}
	out
}

/// The flags of `podup --help` and of every command under it, nested ones too.
fn cli_flags() -> (BTreeSet<String>, Vec<String>) {
	let mut flags = long_flags(&help(&[]));
	let top = subcommands(&help(&[]));
	let mut stack: Vec<Vec<String>> = top.iter().map(|name| vec![name.clone()]).collect();
	while let Some(path) = stack.pop() {
		let args: Vec<&str> = path.iter().map(String::as_str).collect();
		let text = help(&args);
		flags.extend(long_flags(&text));
		for child in subcommands(&text) {
			let mut next = path.clone();
			next.push(child);
			stack.push(next);
		}
	}
	(flags, top)
}

/// The page with the roff escapes that split a flag or a word taken out, so
/// `\fB\-\-no\-start\fR` reads `--no-start`.
fn man_text() -> String {
	let mut text = page().replace("\\-", "-").replace("\\&", "");
	for font in ["\\fB", "\\fI", "\\fR", "\\fP"] {
		text = text.replace(font, "");
	}
	text
}

/// The command names the `.SH COMMANDS` section introduces with `.B`.
fn man_commands() -> BTreeSet<String> {
	page()
		.lines()
		.skip_while(|line| *line != ".SH COMMANDS")
		.skip(1)
		.take_while(|line| !line.starts_with(".SH "))
		.filter_map(|line| line.strip_prefix(".B "))
		.filter_map(|rest| rest.split_whitespace().next())
		.map(str::to_string)
		.collect()
}

#[test]
fn the_man_page_names_no_flag_the_cli_lacks() {
	let (cli, _) = cli_flags();
	let stale: Vec<String> = long_flags(&man_text())
		.into_iter()
		.filter(|flag| {
			!cli.contains(flag) && flag != "--help" && !NOT_IN_HELP.contains(&flag.as_str())
		})
		.collect();
	assert!(
		stale.is_empty(),
		"debian/podup.1 names flags no command accepts:\n  {}",
		stale.join("\n  ")
	);
}

#[test]
fn the_man_page_leaves_out_no_flag_the_cli_has() {
	let (cli, _) = cli_flags();
	let page = long_flags(&man_text());
	let missing: Vec<&String> = cli
		.iter()
		.filter(|flag| {
			!page.contains(*flag)
				&& flag.as_str() != "--help"
				&& !NOT_IN_HELP.contains(&flag.as_str())
		})
		.collect();
	assert!(
		missing.is_empty(),
		"flags the CLI accepts that debian/podup.1 never mentions:\n  {missing:?}"
	);
}

#[test]
fn the_man_page_and_the_cli_list_the_same_commands() {
	let (_, top) = cli_flags();
	let cli: BTreeSet<String> = top.into_iter().collect();
	let page = man_commands();
	let only_cli: Vec<_> = cli.difference(&page).collect();
	let only_page: Vec<_> = page.difference(&cli).collect();
	assert!(
		only_cli.is_empty() && only_page.is_empty(),
		"commands only the CLI has: {only_cli:?}; only debian/podup.1 has: {only_page:?}"
	);
}

/// The page names the Compose Spec, not another vendor's tool, the way the CLI
/// help does. A literal file name (`docker-compose.yaml`) and the variable
/// `DOCKER_HOST` are data a reader has to be able to find, so lines carrying
/// them are allowed; any other mention of the other product is not.
#[test]
fn the_man_page_names_no_other_vendor() {
	let plain = MAN_PAGE
		.replace("\\fB", "")
		.replace("\\fR", "")
		.replace("\\fI", "")
		.replace("\\-", "-");
	let offenders: Vec<&str> = plain
		.lines()
		.filter(|line| {
			let lower = line.to_ascii_lowercase();
			lower.contains("docker")
				&& !lower.contains("docker-compose.y")
				&& !lower.contains("docker-compose.override.")
				&& !line.contains("DOCKER_HOST")
				&& !lower.contains("docker-compatible")
		})
		.collect();
	assert!(
		offenders.is_empty(),
		"debian/podup.1 names another vendor's product:\n  {}",
		offenders.join("\n  ")
	);
}
