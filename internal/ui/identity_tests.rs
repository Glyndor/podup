//! Unit tests for the identity palette and the registry that assigns it, split
//! out to keep the module within the source line limit, the same
//! `tests.rs` split `autostart`, the libpod client and `stats` already use.

use super::*;
use std::collections::HashMap;

/// The whole point of the shared key: `ps` prints `proj-web-1`, `logs`
/// prefixes `web-1`, and the progress lines print `proj-web-1`; all three
/// must resolve to one colour, or the palette is not stable at all.
#[test]
fn every_spelling_of_one_container_gets_one_colour() {
	let _guard = registry_guard();
	set_project("proj");
	let from_ps = identity_style("proj-web-1");
	let from_logs = identity_style("web-1");
	assert_eq!(
		from_ps.render().to_string(),
		from_logs.render().to_string(),
		"the same container must be the same colour in ps and logs"
	);
}

/// Serialises the tests that write the process-wide colour registry.
///
/// `SERVICES` is one static for the whole process, so two tests calling
/// `set_services` on different threads race and each can read the other's
/// registration. That would make both flaky, and a flaky test is a defect
/// rather than noise, so every test that writes the registry takes this
/// first. Tests that only build a local `HashMap` do not need it.
///
/// The lock is poison-tolerant on purpose: a panic in one of these tests must
/// fail that test, not cascade into every other one as a poisoned-lock panic
/// that hides which assertion actually broke.
static COLOUR_REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn registry_guard() -> std::sync::MutexGuard<'static, ()> {
	COLOUR_REGISTRY.lock().unwrap_or_else(|e| e.into_inner())
}

/// `set_services` is what makes registered names spread across the palette:
/// sequential assignment guarantees distinct slots, rather than each name
/// colliding independently under the plain hash. Compared on the slot, which
/// never wraps, so a narrow terminal cannot hide a collision.
#[test]
fn set_services_makes_registered_names_distinct() {
	let _guard = registry_guard();
	// `set_services` is project-keyed; without a project name to register
	// under, every name falls back to the hash (#1517).
	set_project("colourreg-distinct");
	set_services(&[
		"colourreg-alpha".to_string(),
		"colourreg-beta".to_string(),
		"colourreg-gamma".to_string(),
	]);
	let a = service_slot("colourreg-alpha");
	let b = service_slot("colourreg-beta");
	let g = service_slot("colourreg-gamma");
	assert_ne!(a, b, "registered names must not share a colour");
	assert_ne!(b, g, "registered names must not share a colour");
	assert_ne!(a, g, "registered names must not share a colour");
	// Distinct slots are only worth it if they render as distinct colours.
	let style = |slot| slot_to_style(slot, true).render().to_string();
	assert_ne!(
		style(a),
		style(b),
		"distinct slots must render distinct colours"
	);
	assert_ne!(
		style(b),
		style(g),
		"distinct slots must render distinct colours"
	);
	assert_ne!(
		style(a),
		style(g),
		"distinct slots must render distinct colours"
	);
}

/// A label that does not carry the project prefix is left alone: it gets the
/// colour of the service of that name.
#[test]
fn an_unprefixed_label_is_keyed_on_itself() {
	let _guard = registry_guard();
	set_project("proj");
	assert_eq!(identity_slot("web"), service_slot("web"));
}

/// The narrow-terminal palette is a six-entry array, and `slot_to_style`'s
/// narrow branch indexes it with the slot `palette::assign` produced (0
/// through `WIDE_PALETTE.len() - 1`, since `assign` itself wraps at the wide
/// palette's size). It must wrap that again to index the six-entry
/// `SERVICE_PALETTE` safely; without the modulo, a slot of 6 would index out
/// of bounds. Calling `slot_to_style` directly pins that branch
/// without depending on `palette_index`, which used to assert the
/// same condition and is gone.
///
/// Confirmed this fails: with the `% SERVICE_PALETTE.len()` removed from
/// `slot_to_style`'s narrow arm, this test panics with an
/// index-out-of-bounds on slot 6 (`index out of bounds: the len is 6 but
/// the index is 6`), then passes again once the modulo is restored.
#[test]
fn every_wide_palette_slot_indexes_the_narrow_palette_safely() {
	for slot in 0..palette::WIDE_PALETTE.len() {
		let _ = slot_to_style(slot, false);
		let _ = slot_to_style(slot, true);
	}
}

/// Two projects in one process coexist: the first project's colours survive
/// the second project's registration.
///
/// Inversion of the defect pinned in #1518: `SERVICES` was a single
/// process-wide static and `set_services` replaced it rather than merging,
/// so with two `Engine` values alive in one process (helmly-agent's shape,
/// a fleet of projects) the second registration dropped the first project's
/// names. They fell through to `colour_for`'s hash fallback and their colours
/// changed underneath a run that was still going, silently.
///
/// The registry is now keyed by project name: a second registration inserts
/// under a different project key, so the first project's slot stays put.
///
/// The two name sets are chosen so the registered slot and the hash fallback
/// disagree; the second assertion guards that the fixture can actually
/// discriminate: without it the first assertion would hold whether or not the
/// fix were in place, the vacuous shape this file already carries a
/// warning about elsewhere.
#[test]
fn a_second_project_leaves_the_first_projects_colours_alone() {
	let _guard = registry_guard();

	// `set_services` is project-keyed; each project's services live under its
	// own key (#1517). Register the first project's names under `evict-proj-a`.
	set_project("evict-proj-a");
	let first = ["evict-one".to_string(), "evict-two".to_string()];
	set_services(&first);
	let registered = service_slot("evict-one");

	// A second Engine registers its own project under a different key. With
	// project-keying, the first project's slot survives.
	set_project("evict-proj-b");
	set_services(&[
		"evict-other-alpha".to_string(),
		"evict-other-beta".to_string(),
	]);
	let after = service_slot("evict-one");

	// What the first project would get with no registry at all: the hash.
	let unregistered = crate::ui::palette::colour_for("evict-one", &HashMap::new());

	assert_eq!(
		after, registered,
		"the first project's slot must survive the second project's registration \
		 (#1517). If this now differs, the registry stopped being project-keyed \
		 - the first project's entries are being evicted again."
	);
	assert_ne!(
		registered, unregistered,
		"the fixture cannot discriminate: pick names whose registered slot and hash \
		 slot differ, or the assertion above holds whether or not eviction happened"
	);
}
