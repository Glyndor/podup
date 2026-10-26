//! The checks that decide, before the engine is built, whether a command may
//! run without a compose file or with the flags it was given.

pub(in super::super) mod down_by_label;
pub(in super::super) mod label_only;
pub(in super::super) mod update_globals;
