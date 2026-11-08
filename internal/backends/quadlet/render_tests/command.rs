use super::*;

// --- render_command --------------------------------------------------------

#[test]
fn render_command_exec_quotes_args_with_whitespace() {
	let cmd = Command::Exec(vec![
		"server".to_string(),
		"--port".to_string(),
		"9000".to_string(),
	]);
	// Plain arguments pass through unquoted.
	assert_eq!(render_command(&cmd), "server --port 9000");

	let cmd = Command::Exec(vec!["echo".to_string(), "hello world".to_string()]);
	assert_eq!(render_command(&cmd), "echo \"hello world\"");
}

#[test]
fn render_command_exec_multiline_arg_stays_one_line() {
	// A multi-line block-scalar argument (e.g. an `sh -c` script) must be
	// quoted with its newlines C-escaped so the rendered Exec= never spills
	// onto a second physical line or mashes adjacent tokens together.
	let cmd = Command::Exec(vec![
		"sh".to_string(),
		"-c".to_string(),
		"mkdir -p /www\necho hi > /www/index.html\nexec httpd".to_string(),
	]);
	let out = render_command(&cmd);
	assert!(
		!out.contains('\n'),
		"rendered Exec must be a single line: {out}"
	);
	assert_eq!(
		out,
		"sh -c \"mkdir -p /www\\necho hi > /www/index.html\\nexec httpd\""
	);
}

#[test]
fn render_command_exec_escapes_quotes_and_backslashes() {
	let cmd = Command::Exec(vec![
		"sh".to_string(),
		"-c".to_string(),
		"printf '%s' \"a\\b\"".to_string(),
	]);
	let out = render_command(&cmd);
	assert!(!out.contains('\n'));
	// The embedded double quotes and backslash are escaped inside the quoted
	// argument.
	assert!(out.starts_with("sh -c \""));
	assert!(out.contains("\\\""));
	assert!(out.contains("\\\\"));
}

#[test]
fn render_command_shell_neutralizes_newlines() {
	let cmd = Command::Shell("echo a\necho b".to_string());
	let out = render_command(&cmd);
	assert!(!out.contains('\n'));
	assert_eq!(out, "echo a\\necho b");

	// A plain single-line shell command is unchanged.
	assert_eq!(
		render_command(&Command::Shell("echo hi".to_string())),
		"echo hi"
	);
}

#[test]
fn a_semicolon_word_hidden_by_a_control_character_is_still_found() {
	// `;` followed by a control character is not a lone `;` as rendered, and
	// becomes one once the control characters are stripped from the `Exec=`
	// line; systemd then reads it as the end of the command.
	let cmd = Command::Exec(vec!["echo".to_string(), ";\u{1}".to_string()]);
	assert!(has_lone_semicolon_word(&cmd));
	let plain = Command::Exec(vec!["echo".to_string(), "a;b".to_string()]);
	assert!(!has_lone_semicolon_word(&plain));
}
