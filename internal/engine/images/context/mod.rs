//! Build context helpers shared by the build/copy/watch flows:
//! the shared `.dockerignore` / `.containerignore` matcher, the recursive
//! directory walker with pruning, and the single `tar::Builder` site that
//! keeps Podman-readable tar dialect honest across the three call sites.

pub(in crate::engine) mod ignore_patterns;
pub(in crate::engine) mod tar_stream;
pub(in crate::engine) mod walk;

#[cfg(test)]
#[path = "ignore_patterns_tests.rs"]
mod ignore_patterns_tests;

#[cfg(test)]
#[path = "ignore_patterns_time_tests.rs"]
mod ignore_patterns_time_tests;

#[cfg(test)]
#[path = "ignore_patterns_equivalence_tests.rs"]
mod ignore_patterns_equivalence_tests;
