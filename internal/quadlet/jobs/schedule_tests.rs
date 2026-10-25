use super::*;
use crate::compose::types::ScheduleSpec;

fn detailed(
	cron: &str,
	timezone: Option<&str>,
	concurrency: Option<Concurrency>,
	missed_fires: Option<MissedFires>,
) -> Schedule {
	Schedule::Detailed(ScheduleSpec {
		cron: cron.to_string(),
		timezone: timezone.map(str::to_string),
		concurrency,
		missed_fires,
	})
}

#[test]
fn the_short_form_uses_the_specification_defaults() {
	let plan = plan(&Schedule::Cron("0 3 * * *".into())).unwrap();
	assert_eq!(
		plan,
		Plan {
			calendar: "*-*-* 03:00:00".into(),
			timezone: None,
			persistent: true,
		}
	);
}

#[test]
fn forbid_is_exported_and_queue_is_not() {
	let forbid = plan(&detailed(
		"0 3 * * *",
		None,
		Some(Concurrency::Forbid),
		None,
	));
	assert!(forbid.is_ok());
	let queue = plan(&detailed("0 3 * * *", None, Some(Concurrency::Queue), None));
	let reason = queue.unwrap_err();
	assert!(reason.contains("concurrency: queue"), "{reason}");
}

#[test]
fn missed_fires_decides_persistent() {
	let one = plan(&detailed("0 3 * * *", None, None, Some(MissedFires::One))).unwrap();
	assert!(one.persistent);
	let skip = plan(&detailed("0 3 * * *", None, None, Some(MissedFires::Skip))).unwrap();
	assert!(!skip.persistent);
}

#[test]
fn a_timezone_is_kept_only_when_it_is_an_iana_name() {
	let ok = plan(&detailed("0 3 * * *", Some("Europe/Madrid"), None, None)).unwrap();
	assert_eq!(ok.timezone.as_deref(), Some("Europe/Madrid"));
	for zone in ["..", "a b", "a\nb", "/x", "UTC%h", "x:y"] {
		let reason = plan(&detailed("0 3 * * *", Some(zone), None, None)).unwrap_err();
		assert!(reason.contains("timezone"), "{zone:?}: {reason}");
	}
}

#[test]
fn a_cron_outside_the_subset_names_the_reason() {
	let reason = plan(&Schedule::Cron("0 0 1 * 1".into())).unwrap_err();
	assert!(reason.contains("either"), "{reason}");
	let reason = plan(&Schedule::Cron("@reboot".into())).unwrap_err();
	assert!(reason.contains("@reboot"), "{reason}");
}
