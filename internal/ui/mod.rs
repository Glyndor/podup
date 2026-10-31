//! Terminal styling: the single place that decides whether to colour output and
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

pub use anstyle::{AnsiColor, Style};

pub use anstream::ColorChoice;

pub mod progress;
mod render;
mod style;

use render::{header, labelled, lines, table};
use style::{colour, identity, palette, status, styles};

pub use colour::{set_color_choice, stderr_colored, stdout_colored};
pub use header::{print_bold_header, write_bold_header};
pub(crate) use identity::{identity_slot, style_for_slot};
pub use identity::{identity_style, set_project, set_services};
// `service_slot` has one reader outside this module, and it is the log-prefix
// regression test, which compares a raw label against a routed one. A build
// without that test still has to answer the same path, so the re-export stands
// even with nothing behind it.
#[allow(unused_imports)]
pub(crate) use identity::service_slot;
pub use labelled::{
	print_identity_header, print_labelled, print_labelled_neutral, print_labelled_with,
};
pub(crate) use lines::write_progress_line;
pub use lines::{progress_enabled, progress_line, progress_note, result_line, set_progress};
pub use status::action_or_status_style;
pub(crate) use status::paint_status_cell;
pub use styles::{bold, error_style, paint};
pub use table::{fit_cell, sanitize_cell, Table};
