//! Changing whether containers run: start, stop, restart, and pause/unpause.

mod pause;
mod restart;
mod start;
mod stop;
pub use stop::ContainerSnapshot;

#[cfg(all(test, unix))]
mod tests;
