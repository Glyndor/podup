//! An empty profile name is the profile that is active when none is.
//!
//! `profiles: ["${PROFILE:-}"]` with the variable unset leaves `""`. Docker
//! Compose starts such a service on a plain `up` and leaves it out once any
//! profile is named. Reading `""` as a profile nobody can name left it inactive
//! always, and a file whose services were all in that position failed with
//! "no services defined".

use super::*;
use crate::compose::types::Service;

fn with_profiles(profiles: &[&str]) -> Service {
	Service {
		profiles: profiles.iter().map(|p| p.to_string()).collect(),
		..Default::default()
	}
}

#[test]
fn a_service_whose_only_profile_is_empty_runs_on_a_plain_up() {
	let active: HashSet<String> = HashSet::new();
	assert!(service_in_profiles(&with_profiles(&[""]), &active));
}

#[test]
fn a_service_with_an_empty_profile_among_others_runs_on_a_plain_up() {
	let active: HashSet<String> = HashSet::new();
	assert!(service_in_profiles(&with_profiles(&["", "p"]), &active));
}

#[test]
fn a_named_profile_still_needs_to_be_active() {
	let off: HashSet<String> = HashSet::new();
	assert!(!service_in_profiles(&with_profiles(&["p"]), &off));
	let on: HashSet<String> = ["p".to_string()].into();
	assert!(service_in_profiles(&with_profiles(&["p"]), &on));
}

#[test]
fn naming_a_profile_leaves_out_a_service_that_only_has_the_empty_one() {
	let on: HashSet<String> = ["p".to_string()].into();
	assert!(!service_in_profiles(&with_profiles(&[""]), &on));
}

#[test]
fn naming_a_profile_keeps_a_service_that_lists_it_beside_the_empty_one() {
	let on: HashSet<String> = ["p".to_string()].into();
	assert!(service_in_profiles(&with_profiles(&["", "p"]), &on));
}
