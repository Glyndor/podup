//! Log streaming: what `logs` prints, and the rules every stream shares.
//!
//! `filters` validates `--tail`/`--since`/`--until`, `prefix` tags each line
//! with `{service} | `, `stream` holds the label, the query string and how a
//! stream is classified when it ends, and `with_display` is the command that
//! drives them. The option structs stay in the parent `options.rs` because
//! `images` shares them.

mod filters;
mod prefix;
mod stream;
mod with_display;

pub(in crate::engine) use stream::stream_broke_mid_output;
