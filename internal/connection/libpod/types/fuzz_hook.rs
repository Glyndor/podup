//! Entry point for the fuzz harness: every response type the client decodes
//! from the daemon's JSON, reachable by an index so one target covers them all.

use serde::de::DeserializeOwned;

use super::container::{ContainerInspect, ContainerListEntry, TopResponse};
use super::exec::{ExecCreateResponse, ExecInspect};
use super::image::{BuildOutput, ImageInspect, ImagePullProgress};
use super::pod::PodInspect;
use super::volume::SystemDf;

/// How many response types [`decode`] knows.
pub const KINDS: u8 = 10;

fn parses<T: DeserializeOwned>(bytes: &[u8]) -> bool {
	serde_json::from_slice::<T>(bytes).is_ok()
}

/// Decode `bytes` as the response type numbered `kind` (taken modulo
/// [`KINDS`]), returning whether it was accepted. A refusal is an ordinary
/// outcome; the harness only cares that nothing panics.
pub fn decode(kind: u8, bytes: &[u8]) -> bool {
	match kind % KINDS {
		0 => parses::<ContainerInspect>(bytes),
		1 => parses::<Vec<ContainerListEntry>>(bytes),
		2 => parses::<TopResponse>(bytes),
		3 => parses::<ExecCreateResponse>(bytes),
		4 => parses::<ExecInspect>(bytes),
		5 => parses::<ImagePullProgress>(bytes),
		6 => parses::<BuildOutput>(bytes),
		7 => parses::<ImageInspect>(bytes),
		8 => parses::<PodInspect>(bytes),
		_ => parses::<SystemDf>(bytes),
	}
}
