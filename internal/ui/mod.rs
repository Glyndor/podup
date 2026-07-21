//! Terminal styling — the single place that decides whether to colour output and
//! holds the palette, so colour stays consistent and always honours `--ansi`,
//! `NO_COLOR`, and whether the target stream is a TTY.
//!
//! Two kinds of sink exist in podup:
//! - Sinks written through [`anstream`] (e.g. [`anstream::stdout`]) strip the
//!   ANSI codes themselves when colour is off, so callers may always emit codes.
//! - Raw sinks (the tracing writer, the log-prefixer's borrowed stdout) do not
//!   strip, so those callers gate on `stdout_colored`/`stderr_colored`.
//!
//! `set_color_choice` also constructs the auto streams once so anstream enables
//! virtual-terminal processing on Windows for the rest of the process.

use std::io::IsTerminal;

pub use anstyle::{AnsiColor, Style};

pub use anstream::ColorChoice;

mod table;
pub use table::{fit_cell, Table};

/// Apply the resolved colour choice process-wide and enable Windows VT once.
///
/// anstream's `stdout()`/`stderr()` then honour this choice together with
/// `NO_COLOR`/`CLICOLOR` and TTY detection.
pub fn set_color_choice(choice: ColorChoice) {
	choice.write_global();
	// Constructing the auto streams enables virtual-terminal processing on
	// Windows so raw-sink callers can emit ANSI safely; a no-op elsewhere.
	let _ = anstream::AutoStream::auto(std::io::stdout());
	let _ = anstream::AutoStream::auto(std::io::stderr());
}

/// `NO_COLOR` is set to a non-empty value (the no-color.org convention).
fn no_color() -> bool {
	std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// Whether a stream that is (or isn't) a TTY should be coloured under `choice`.
/// `Auto` defers to the TTY and `NO_COLOR`. Pure (takes the choice as an argument)
/// so the policy is tested without mutating the process-global choice.
fn colored_with(choice: ColorChoice, is_terminal: bool) -> bool {
	match choice {
		ColorChoice::Always | ColorChoice::AlwaysAnsi => true,
		ColorChoice::Never => false,
		ColorChoice::Auto => is_terminal && !no_color(),
	}
}

/// Whether a stream that is (or isn't) a TTY should be coloured under the global
/// choice.
fn colored(is_terminal: bool) -> bool {
	colored_with(ColorChoice::global(), is_terminal)
}

/// Whether a raw (non-anstream) write to stdout should embed ANSI codes.
pub fn stdout_colored() -> bool {
	colored(std::io::stdout().is_terminal())
}

/// Whether a raw (non-anstream) write to stderr should embed ANSI codes.
pub fn stderr_colored() -> bool {
	colored(std::io::stderr().is_terminal())
}

/// Process-wide switch for user-facing lifecycle progress output. Off by default
/// so the library (consumed by other crates) stays silent unless asked; the CLI
/// turns it on for the human-facing lifecycle commands. Progress is written to
/// stderr (matching docker compose) so stdout stays a clean pipe for scripting
/// and machine/JSON output.
static PROGRESS_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The project name, so an identity colour can be keyed on the same label
/// everywhere.
///
/// `logs` prefixes lines with the project-stripped `web-1`; `ps` prints the full
/// container name `proj-web-1`; the progress lines print the full name too.
/// Hashing whatever string each site happens to hold gives the same container a
/// different colour in each command — which defeats the point of a stable
/// palette. Stripping the project first makes one container one colour.
static PROJECT: std::sync::RwLock<String> = std::sync::RwLock::new(String::new());

/// Record the project name for identity colouring. Set once per invocation.
pub fn set_project(name: &str) {
	if let Ok(mut slot) = PROJECT.write() {
		name.clone_into(&mut slot);
	}
}

/// The stable identity colour for a container or service label, keyed on the
/// label with the project prefix removed.
///
/// Callers pass whatever they display — `proj-web-1`, `web-1`, `web` — and get
/// the same colour for the same container, which is what makes `ps`, `logs`,
/// `stats` and the progress lines agree.
pub fn identity_style(label: &str) -> Style {
	let key = PROJECT
		.read()
		.ok()
		.and_then(|p| {
			(!p.is_empty())
				.then(|| label.strip_prefix(&format!("{p}-")).map(str::to_string))
				.flatten()
		})
		.unwrap_or_else(|| label.to_string());
	service_style(&key)
}

/// Enable or disable user-facing lifecycle progress output process-wide. The CLI
/// enables it for the lifecycle commands; embedders that want podup silent leave
/// it off (the default).
pub fn set_progress(enabled: bool) {
	PROGRESS_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Whether user-facing lifecycle progress output is currently enabled.
pub fn progress_enabled() -> bool {
	PROGRESS_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Emit a concise per-resource lifecycle progress line to stderr, mirroring
/// docker compose's `Container <name>  <Action>` progress stream (which also
/// goes to stderr). `kind` is the resource noun (`Container`, `Network`,
/// `Volume`, `Image`); `action` is the past-tense verb (`Started`, `Removed`,
/// …), tinted green on a colour sink. A no-op unless [`set_progress`] enabled
/// progress output, so stdout consumers and machine output are never polluted.
pub fn progress_line(kind: &str, name: &str, action: &str) {
	use std::io::Write;
	if !progress_enabled() {
		return;
	}
	let verb = action_style(action);
	let ident = identity_style(name);
	// anstream::stderr strips the ANSI codes itself when colour is off.
	let _ = writeln!(
		anstream::stderr(),
		" {kind} {}{name}{}  {}{action}{}",
		ident.render(),
		ident.render_reset(),
		verb.render(),
		verb.render_reset()
	);
}

/// The colour band for a lifecycle verb.
///
/// Every verb used to be the same green, so `Volume data Removed` — which
/// destroys data and cannot be undone — was styled exactly like `Started`. The
/// bands say what kind of thing happened: something now exists (green),
/// something stopped but survives (yellow), something is gone (red), nothing
/// changed (dim).
fn action_style(action: &str) -> Style {
	let a = action.to_ascii_lowercase();
	if a.starts_with("remov") || a.starts_with("kill") || a.starts_with("delet") {
		Style::new().fg_color(Some(AnsiColor::Red.into()))
	} else if a.starts_with("stop") || a.starts_with("paus") || a.starts_with("restart") {
		Style::new().fg_color(Some(AnsiColor::Yellow.into()))
	} else if a.starts_with("exist") || a.starts_with("running") || a.starts_with("skip") {
		Style::new().dimmed()
	} else {
		Style::new().fg_color(Some(AnsiColor::Green.into()))
	}
}

/// Emit a plain user-facing progress note to stderr (e.g. a "nothing to do"
/// line). A no-op unless [`set_progress`] enabled progress output.
pub fn progress_note(msg: &str) {
	use std::io::Write;
	if !progress_enabled() {
		return;
	}
	let _ = writeln!(anstream::stderr(), "{msg}");
}

/// Print a result line to stdout — used by `run -d` to echo the started
/// container's name so scripts capturing stdout get the id, matching
/// `docker compose run -d`. A no-op unless [`set_progress`] enabled progress
/// output, so embedders and machine paths stay silent.
///
/// The result line is the one thing scripts capture from stdout, so a failed
/// write is a real error (exit non-zero), not something to swallow — except a
/// broken pipe, which follows the process-wide quiet-exit convention.
pub fn result_line(msg: &str) -> std::io::Result<()> {
	use std::io::Write;
	if !progress_enabled() {
		return Ok(());
	}
	match writeln!(anstream::stdout(), "{msg}") {
		Err(e) if e.kind() != std::io::ErrorKind::BrokenPipe => Err(e),
		_ => Ok(()),
	}
}

/// Bold — table headers and emphasis.
pub fn bold() -> Style {
	Style::new().bold()
}

/// Print `cols` as a bold table header through an anstream sink, so the styling
/// is stripped automatically when colour is off. The single place every list
/// command renders its header, keeping them consistent.
pub fn print_bold_header(cols: &str) {
	use std::io::Write;
	let s = bold();
	let _ = writeln!(
		anstream::stdout(),
		"{}{cols}{}",
		s.render(),
		s.render_reset()
	);
}

/// Bold red — the `error:` label.
pub fn error_style() -> Style {
	Style::new().bold().fg_color(Some(AnsiColor::Red.into()))
}

/// Wrap `text` in `style` when `enabled`, else return it unchanged. For raw sinks
/// that do not strip; anstream sinks should pass the codes through directly.
pub fn paint(style: Style, text: &str, enabled: bool) -> String {
	if enabled {
		format!("{}{text}{}", style.render(), style.render_reset())
	} else {
		text.to_string()
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

/// Stable palette index for a service name — the same name always maps to the
/// same colour (FNV-1a over the bytes, modulo the palette size).
fn palette_index(name: &str) -> usize {
	let mut h: u64 = 0xcbf2_9ce4_8422_2325;
	for b in name.bytes() {
		h ^= u64::from(b);
		h = h.wrapping_mul(0x0100_0000_01b3);
	}
	(h % SERVICE_PALETTE.len() as u64) as usize
}

/// The stable colour for a service's aggregated-log prefix.
pub fn service_style(name: &str) -> Style {
	Style::new().fg_color(Some(SERVICE_PALETTE[palette_index(name)].into()))
}

/// Whether an `Exited (N)` / `exited(N)` label reports a clean finish.
///
/// A container that ran to completion is not a failure, and colouring it red
/// says it is. One-shot services — migrations, seeds, a `command` that simply
/// ends — spend their whole life in this state, so red here is not a rare
/// cosmetic slip but the normal reading of a healthy project.
fn is_clean_exit(lower: &str) -> bool {
	// `exited (0)` and `exited(0)`, but not `exited (07)` or `exited (10)`.
	let Some(rest) = lower
		.split_once("exit")
		.map(|(_, r)| r.trim_start_matches(['e', 'd', ' ', '(']))
	else {
		return false;
	};
	rest.starts_with('0')
		&& !rest
			.trim_start_matches('0')
			.starts_with(|c: char| c.is_ascii_digit())
}

/// The semantic colour for a container status word, or `None` for an unknown one
/// (left uncoloured). Green = up/healthy, red = failed/dead/unhealthy, yellow =
/// paused/(re)starting/stopping, dim = created or finished cleanly. Matches on
/// substrings so it handles the bare state (`running`), Podman's verbose
/// `Status` (`Up 2 minutes`, `Exited (1)`), and systemd's vocabulary, which
/// `autostart status` reports.
fn status_style(status: &str) -> Option<Style> {
	let s = status.to_ascii_lowercase();
	// Checked before the red arm: `Exited (0)` contains "exit" and would
	// otherwise be indistinguishable from `Exited (7)`.
	if s.contains("exit") && is_clean_exit(&s) {
		return Some(Style::new().dimmed());
	}
	let colour = if s.contains("unhealthy")
		|| s.contains("exit")
		|| s.contains("dead")
		|| s.contains("failed")
		|| s.contains("not-found")
		|| s.contains("error")
	{
		AnsiColor::Red
	} else if s.contains("running")
		|| s.contains("healthy")
		|| s.starts_with("up")
		|| s == "active"
		|| s == "enabled"
		|| s == "yes"
	{
		AnsiColor::Green
	} else if s.contains("paus")
		|| s.contains("restart")
		|| s.contains("starting")
		|| s.contains("stopping")
		|| s.contains("removing")
		|| s.contains("activating")
	{
		AnsiColor::Yellow
	} else if s.contains("created") || s == "inactive" || s == "disabled" || s == "no" {
		return Some(Style::new().dimmed());
	} else {
		return None;
	};
	Some(Style::new().fg_color(Some(colour.into())))
}

/// Colourise a status cell, tinting each comma-separated segment on its own.
///
/// `ls` reports a project as `running(1), exited(1)`, and styling that as one
/// string made the first matching substring win: `exit` came first, so a project
/// with a service up rendered **entirely red** — visually identical to one that
/// is completely dead. Splitting first means each state carries its own colour
/// and the mixed case reads as mixed.
///
/// Trailing padding is preserved verbatim so column alignment is untouched.
pub(crate) fn paint_status_cell(padded: &str) -> String {
	let trimmed = padded.trim_end();
	let pad = &padded[trimmed.len()..];
	let body = trimmed
		.split(", ")
		.map(|seg| match status_style(seg) {
			Some(style) => paint(style, seg, true),
			None => seg.to_string(),
		})
		.collect::<Vec<_>>()
		.join(", ");
	format!("{body}{pad}")
}

/// Render a container `status` left-padded to `width`, colourised by its meaning
/// when stdout is a colour sink. The padding is applied first so the colour codes
/// (zero display width) never disturb column alignment.
pub fn status_cell(status: &str, width: usize) -> String {
	let padded = format!("{status:<width$}");
	match status_style(status) {
		Some(style) => paint(style, &padded, stdout_colored()),
		None => padded,
	}
}

#[cfg(test)]
mod status_colour_tests {
	use super::*;

	/// The bug this exists to stop: `ls` reports `running(1), exited(1)`, and
	/// styling it as one string let the first matching substring win — `exit`
	/// came first, so a project with a service up rendered entirely red,
	/// indistinguishable from one that is completely dead.
	#[test]
	fn a_mixed_project_is_not_painted_as_one_state() {
		let out = paint_status_cell("running(1), exited(1)");
		let (running, exited) = out.split_once(", ").expect("both segments survive");
		assert_ne!(
			running.replace("running(1)", ""),
			exited.replace("exited(1)", ""),
			"each state must carry its own colour: {out:?}"
		);
	}

	/// A container that ran to completion is not a failure. One-shot services —
	/// migrations, seeds, a `command` that simply ends — live in this state.
	#[test]
	fn a_clean_exit_is_not_red() {
		let red = Style::new().fg_color(Some(AnsiColor::Red.into()));
		let clean = paint_status_cell("Exited (0)");
		assert!(
			!clean.contains(&red.render().to_string()),
			"a zero exit must not be red: {clean:?}"
		);
		let failed = paint_status_cell("Exited (7)");
		assert!(
			failed.contains(&red.render().to_string()),
			"a non-zero exit must stay red: {failed:?}"
		);
	}

	/// Digits after the first must not be mistaken for a clean exit.
	#[test]
	fn only_a_bare_zero_counts_as_clean() {
		assert!(is_clean_exit("exited (0)"));
		assert!(is_clean_exit("exited(0)"));
		assert!(!is_clean_exit("exited (10)"));
		assert!(!is_clean_exit("exited (07)"));
	}

	/// Padding is what keeps columns aligned, so colourising must not eat it.
	#[test]
	fn trailing_padding_survives_colourising() {
		let out = paint_status_cell("running   ");
		assert!(out.ends_with("   "), "{out:?}");
	}

	/// systemd's vocabulary reaches this through `autostart status`.
	#[test]
	fn systemd_states_are_coloured() {
		for word in ["active", "inactive", "failed", "not-found", "enabled"] {
			assert!(status_style(word).is_some(), "{word} should carry a colour");
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn service_colour_is_stable_per_name() {
		// Same name → same index every call; different names spread across the
		// palette (not all collapsed to one colour).
		assert_eq!(palette_index("web"), palette_index("web"));
		let distinct: std::collections::HashSet<usize> =
			["web", "db", "cache", "worker", "proxy", "queue"]
				.iter()
				.map(|n| palette_index(n))
				.collect();
		assert!(distinct.len() > 1, "palette should spread service names");
		assert!(palette_index("web") < SERVICE_PALETTE.len());
	}

	#[test]
	fn paint_gates_on_enabled() {
		let plain = paint(bold(), "hi", false);
		assert_eq!(plain, "hi");
		let coloured = paint(bold(), "hi", true);
		assert!(coloured.contains("hi"));
		assert!(coloured.len() > "hi".len(), "enabled paint adds ANSI codes");
		assert!(coloured.starts_with('\u{1b}'), "starts with an ESC");
	}

	#[test]
	fn colour_choice_resolution() {
		// Pure resolution — never touches the process-global choice, so it can't
		// race the production code (LinePrefixer/status_cell) that reads it.
		temp_env::with_var_unset("NO_COLOR", || {
			assert!(!colored_with(ColorChoice::Never, true));
			assert!(colored_with(ColorChoice::Always, false));
			assert!(colored_with(ColorChoice::Auto, true));
			assert!(!colored_with(ColorChoice::Auto, false));
		});
		// NO_COLOR forces plain in Auto, regardless of the TTY.
		temp_env::with_var("NO_COLOR", Some("1"), || {
			assert!(!colored_with(ColorChoice::Auto, true));
			// ...but an explicit `always` still overrides NO_COLOR.
			assert!(colored_with(ColorChoice::Always, true));
		});
	}

	#[test]
	fn status_style_is_semantic() {
		assert_ne!(status_style("running"), status_style("exited (1)"));
		assert_ne!(status_style("unhealthy"), status_style("healthy"));
		assert!(status_style("Up 2 minutes").is_some());
		assert!(status_style("paused").is_some());
		assert!(status_style("created").is_some());
		assert!(status_style("weird-state").is_none());
	}

	#[test]
	fn progress_toggle_is_observable() {
		// Off by default-or-restored; toggling flips the observable state. Restore
		// afterwards so the process-global flag does not leak into other tests.
		let prev = progress_enabled();
		set_progress(false);
		assert!(!progress_enabled());
		set_progress(true);
		assert!(progress_enabled());
		set_progress(prev);
	}

	#[test]
	fn status_cell_pads_and_keeps_status() {
		let cell = status_cell("ok", 6);
		assert!(cell.contains("ok"));
		// At least the requested width (colour codes, if any, only add length).
		assert!(cell.len() >= 6);
	}
}

#[cfg(test)]
mod identity_key_tests {
	use super::*;

	/// The whole point of the shared key: `ps` prints `proj-web-1`, `logs`
	/// prefixes `web-1`, and the progress lines print `proj-web-1` — all three
	/// must resolve to one colour, or the palette is not stable at all.
	#[test]
	fn every_spelling_of_one_container_gets_one_colour() {
		set_project("proj");
		let from_ps = identity_style("proj-web-1");
		let from_logs = identity_style("web-1");
		assert_eq!(
			from_ps.render().to_string(),
			from_logs.render().to_string(),
			"the same container must be the same colour in ps and logs"
		);
	}

	/// A label that does not carry the project prefix is left alone.
	#[test]
	fn an_unprefixed_label_is_keyed_on_itself() {
		set_project("proj");
		assert_eq!(
			identity_style("web").render().to_string(),
			service_style("web").render().to_string()
		);
	}
}
