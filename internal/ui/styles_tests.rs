use super::*;

#[test]
fn paint_gates_on_enabled() {
	let plain = paint(bold(), "hi", false);
	assert_eq!(plain, "hi");
	let coloured = paint(bold(), "hi", true);
	assert!(coloured.contains("hi"));
	assert!(coloured.len() > "hi".len(), "enabled paint adds ANSI codes");
	assert!(coloured.starts_with('\u{1b}'), "starts with an ESC");
}
