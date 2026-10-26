//! Sending a signal to containers: `kill` and the validation of its `-s` value.

mod kill;
pub(in super::super) mod kill_signal;

#[cfg(test)]
mod kill_signal_tests;
