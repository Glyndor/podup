//! The checks that decide, before the engine is built, whether a command may
//! run without a compose file or with the flags it was given.

pub(crate) mod down_by_label;
pub(crate) mod label_only;
pub(crate) mod update_globals;
