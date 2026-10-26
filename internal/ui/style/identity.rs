//! Identity colours: which colour a service is drawn in, and the registry that
//! hands each one its own.
//!
//! The promise the whole palette rests on is that the same container is the same
//! colour everywhere: `ps` prints `proj-web-1`, `logs` prefixes `web-1`, and the
//! progress lines print the full name again. That only holds if every site keys
//! on the label with the project prefix and the replica index removed, which is
//! what [`identity_slot`] does and what [`identity_style`] renders.
//!
//! Registration is per project rather than process-wide, so two `Engine` values
//! alive in one process (helmly-agent's fleet shape) do not evict each other's
//! colours (#1517).

use super::super::palette;
use super::super::AnsiColor;
use super::super::Style;

/// The project name, so an identity colour can be keyed on the same label
/// everywhere.
///
/// `logs` prefixes lines with the project-stripped `web-1`; `ps` prints the full
/// container name `proj-web-1`; the progress lines print the full name too.
/// Hashing whatever string each site happens to hold gives the same container a
/// different colour in each command, which defeats the point of a stable
/// palette. Stripping the project first makes one container one colour.
static PROJECT: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// The pre-formatted `"{project}-"` prefix, cached so [`identity_slot`] does
/// not reallocate it on every call. Set in tandem with [`PROJECT`] by
/// [`set_project`]; read by [`identity_slot`]'s no-registration fallback path
/// so `proj-web-1` and `web-1` still collapse to one hash slot when no
/// project has called `set_services` yet. The main registered lookup iterates
/// the per-project prefix entries in [`SERVICES`] directly, so this cache
/// only matters when nothing has been registered.
static PROJECT_PREFIX: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// The service-to-colour map, keyed by project name so two `Engine` values
/// alive in one process (helmly-agent's fleet shape) coexist: a second
/// `set_services` inserts under a different project key and the first
/// project's slots survive (#1517).
///
/// Empty until [`set_services`] is called, which is why [`identity_slot`] and
/// [`service_slot`] fall back to a hash rather than requiring registration.
static SERVICES: std::sync::RwLock<
	Option<std::collections::HashMap<String, std::collections::HashMap<String, usize>>>,
> = std::sync::RwLock::new(None);

/// Record the project name for identity colouring. Set once per invocation
/// per project: two `Engine` values alive in one process must each register
/// their own project name, and the last `set_project` wins for the global
/// `PROJECT`, but every project's services live under its own key in the
/// service registry, so neither evicts the other.
pub fn set_project(name: &str) {
	if let Ok(mut slot) = PROJECT.write() {
		name.clone_into(&mut slot);
	}
	if let Ok(mut prefix_slot) = PROJECT_PREFIX.write() {
		if name.is_empty() {
			prefix_slot.clear();
		} else {
			let mut prefixed = String::with_capacity(name.len() + 1);
			prefixed.push_str(name);
			prefixed.push('-');
			*prefix_slot = prefixed;
		}
	}
}

/// Record this project's service names so each gets its own identity colour.
///
/// Call once per project per invocation, after the compose file is parsed
/// (or, for the label-only teardown in
/// `engine::lifecycle::down::label::Engine::down_by_label`, after the live
/// container listing is fetched). Without a prior [`set_project`] for the
/// same project this is a no-op: registration has no project key to live
/// under. Every CLI command and the engine call site already calls
/// [`set_project`] first. Without any registration every label falls back
/// to the per-label hash, which still works; it just cannot promise two
/// services differ.
pub fn set_services(names: &[String]) {
	let project = match PROJECT.read() {
		Ok(p) if !p.is_empty() => p.clone(),
		_ => {
			// Returning quietly would leave every label on the hash fallback
			// with nothing to say why: colours would still appear, just not
			// the assigned ones, which is the shape of failure nobody
			// notices. The CLI always calls set_project first; a library
			// consumer has no such habit, and this is the line that tells
			// them.
			tracing::warn!(
				"identity colours not registered: set_project must be called before set_services"
			);
			return;
		}
	};
	if let Ok(mut slot) = SERVICES.write() {
		let map = slot.get_or_insert_with(std::collections::HashMap::new);
		map.insert(project, palette::assign(names));
	}
}

/// The stable identity colour for a container or service label, keyed on the
/// label with the project prefix removed.
///
/// Callers pass whatever they display (`proj-web-1`, `web-1`, `web`) and get
/// the same colour for the same container, which is what makes `ps`, `logs`,
/// `stats` and the progress lines agree.
pub fn identity_style(label: &str) -> Style {
	slot_to_style(identity_slot(label), palette::wide_palette_available())
}

/// The palette slot backing [`identity_style`]. See [`service_slot`] for why
/// this exists as its own, unrendered step.
///
/// First tries every project registered in [`SERVICES`], stripping each
/// project's `"{name}-"` prefix in turn, so a label from a project whose
/// [`set_project`] was overwritten by another engine in the same process
/// still resolves correctly (#1517). If no project matched, falls back to
/// the legacy [`PROJECT_PREFIX`]-stripped lookup so `proj-web-1` and
/// `web-1` collapse to one hash slot in the no-registration case
/// (`every_spelling_of_one_container_gets_one_colour` pins that contract).
pub(crate) fn identity_slot(label: &str) -> usize {
	if let Ok(slot) = SERVICES.read() {
		if let Some(map) = slot.as_ref() {
			for (project, services) in map.iter() {
				if project.is_empty() {
					continue;
				}
				let prefix = format!("{project}-");
				if let Some(stripped) = label.strip_prefix(prefix.as_str()) {
					let key = strip_replica_suffix(stripped);
					if let Some(&slot) = services.get(key) {
						return slot;
					}
				}
			}
		}
	}
	// No project's services matched. Strip the currently-set project prefix
	// (cached in PROJECT_PREFIX so the per-call `format!` is not re-paid)
	// and look up the bare key, the legacy single-engine behaviour that
	// `every_spelling_of_one_container_gets_one_colour` pins (#1364).
	let prefix = PROJECT_PREFIX
		.read()
		.ok()
		.map(|p| p.clone())
		.unwrap_or_default();
	let stripped = if prefix.is_empty() {
		label.to_string()
	} else {
		label
			.strip_prefix(prefix.as_str())
			.unwrap_or(label)
			.to_string()
	};
	service_slot(strip_replica_suffix(&stripped))
}

/// Drop a trailing `-N` replica index, so every surface hashes the same key.
///
/// Without this the promise above held only within one command: `ps` keys a
/// container as `web-1` while `images` keys the same service as `web`, so one
/// service came out in two different colours depending on which command printed
/// it. Measured before the fix, one project, one invocation: `migrate` was blue
/// in `ps` and light magenta in `images`.
///
/// Only an all-digit suffix is stripped, so a service genuinely named `api-2`
/// keeps its own identity rather than colliding with `api`.
fn strip_replica_suffix(key: &str) -> &str {
	match key.rsplit_once('-') {
		Some((head, tail))
			if !head.is_empty() && !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) =>
		{
			head
		}
		_ => key,
	}
}

/// Identity colours cycled through for per-service log prefixes. Deliberately
/// excludes red/green/yellow, which are reserved for status/severity meaning, so
/// a service prefix is never mistaken for an error/ok/warning signal.
const SERVICE_PALETTE: [AnsiColor; 6] = [
	AnsiColor::Cyan,
	AnsiColor::Magenta,
	AnsiColor::Blue,
	AnsiColor::BrightCyan,
	AnsiColor::BrightMagenta,
	AnsiColor::BrightBlue,
];

/// The palette slot backing a service's identity colour, before it is
/// rendered into a [`Style`] for whichever palette the terminal supports.
///
/// Looks up `name` in every project's services registered in [`SERVICES`],
/// first match wins. Two `Engine` values alive in one process own separate
/// maps under separate project keys, so neither evicts the other (#1517).
/// Falls back to the per-label hash when nothing is registered, the same
/// fallback [`palette::colour_for`](crate::ui::palette::colour_for) has always
/// used for an undeclared label.
///
/// Split out so a routing regression can be asserted on the slot itself: the
/// narrow (6-colour) fallback wraps a slot index mod 6, so two different
/// wide-palette slots can render as the same `Style` there. A test comparing
/// rendered `Style`s can miss exactly the collision it exists to catch on a
/// terminal that never announces the wide palette; the slot never wraps, so
/// it stays a real comparison on both.
pub(crate) fn service_slot(name: &str) -> usize {
	if let Ok(slot) = SERVICES.read() {
		if let Some(map) = slot.as_ref() {
			for services in map.values() {
				if let Some(&slot) = services.get(name) {
					return slot;
				}
			}
		}
	}
	palette::colour_for(name, &std::collections::HashMap::new())
}

/// The style for a palette slot, from whichever palette the terminal supports.
///
/// The palette choice is a parameter so both branches are testable: the real
/// decision reads a process-cached environment probe, and a test that flipped
/// it would pass or fail on test scheduling.
///
/// The narrow branch takes a slot assigned against the WIDE palette's size, so
/// it must wrap again: indexing a six-element array with a slot up to 19 is
/// the out-of-bounds bug the old hash's `assert!` used to guard.
fn slot_to_style(slot: usize, wide: bool) -> Style {
	if wide {
		Style::new().fg_color(Some(palette::wide_colour(slot)))
	} else {
		Style::new().fg_color(Some(SERVICE_PALETTE[slot % SERVICE_PALETTE.len()].into()))
	}
}

/// Render a palette slot (from [`identity_slot`]/[`service_slot`]) into a
/// [`Style`] for whichever palette the terminal supports right now.
///
/// The `pub(crate)` counterpart to [`identity_style`] for a caller that needs
/// to resolve its *own* slot (e.g. the log-prefix module, which must never
/// let its routing collapse into a raw per-label hash; see
/// `engine::query::logs::prefix::prefix_slot`) rather than one of the two
/// label-keyed lookups here.
pub(crate) fn style_for_slot(slot: usize) -> Style {
	slot_to_style(slot, palette::wide_palette_available())
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
