//! The command bodies that need more than a dispatch arm: `autostart`,
//! `generate` and `help`.

pub(crate) mod autostart;
pub(crate) mod generate;
pub(crate) mod help;

#[cfg(test)]
mod generate_tests;
