//! The two stages of `run`: the commands that answer before the compose file is
//! parsed, and the ones that need it.

pub(in super::super) mod post_parse;
pub(in super::super) mod short_circuits;
