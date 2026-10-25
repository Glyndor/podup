//! Parse tests for features present in the type system but not previously
//! covered. Split into part files to keep each under the source line limit.

#[path = "coverage/deploy_build/mod.rs"]
mod deploy_build;
#[path = "coverage/network_volume/mod.rs"]
mod network_volume;
#[path = "coverage/resources_secrets/mod.rs"]
mod resources_secrets;
