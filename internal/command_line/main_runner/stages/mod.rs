//! The two stages of `run`: the commands that answer before the compose file is
//! parsed, and the ones that need it.

pub(crate) mod post_parse;
pub(crate) mod short_circuits;
