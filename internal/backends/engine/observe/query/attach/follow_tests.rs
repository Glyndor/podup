use super::render_line;

#[test]
fn the_prefix_is_printed_unless_it_is_turned_off() {
	assert_eq!(render_line("web-1", false, "hi\n"), "web-1 | hi\n");
	assert_eq!(render_line("web-1", true, "hi\n"), "hi\n");
}
