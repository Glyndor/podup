use super::*;

#[test]
fn formats_docker_compat_shape() {
	let ev = json!({
		"Type": "container",
		"Action": "start",
		"Actor": { "Attributes": { "name": "web-1" } },
		"time": 0,
	});
	// Columns are fixed-width now (#1248): TIME leads, `container` fills TYPE
	// exactly, `start` is padded out to ACTION, and NAME is the trailing raw
	// column.
	//
	// The TIME cell is asserted by width rather than by value: it renders
	// the reader's wall clock, so a fixed string would pass on a machine at
	// -05:00 and fail on a runner at UTC. `crate::timestamp` pins what goes
	// in the cell; this pins that the columns after it land where the header
	// says.
	let out = format_event(&ev, false);
	assert_eq!(
		&out[TIME_WIDTH..],
		" container start          web-1",
		"columns after TIME drifted: {out:?}"
	);
}

#[test]
fn formats_libpod_native_shape() {
	let ev = json!({ "Type": "container", "status": "die", "id": "abc123", "time": 0 });
	let out = format_event(&ev, false);
	assert_eq!(
		&out[TIME_WIDTH..],
		" container die            abc123",
		"columns after TIME drifted: {out:?}"
	);
}

/// The table form must render `died` as `die` and `remove` as `delete`,
/// the docker-compat verbs podup has always printed (#1914).
#[test]
fn table_form_renders_libpod_verbs_as_docker_compat() {
	let died = json!({ "Type": "container", "status": "died", "id": "web-1", "time": 0 });
	let out = format_event(&died, false);
	assert!(
		out.contains("die"),
		"libpod `died` must render as docker-compat `die`: {out:?}"
	);
	assert!(
		!out.contains("died"),
		"libpod `died` must not appear in the table form: {out:?}"
	);

	let remove = json!({ "Type": "image", "status": "remove", "id": "img-1", "time": 0 });
	let out = format_event(&remove, false);
	assert!(
		out.contains("delete"),
		"libpod `remove` must render as docker-compat `delete`: {out:?}"
	);
}
