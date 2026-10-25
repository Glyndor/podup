//! What a job's unit is ordered after, and the jobs that are not exported at
//! all because their project or their definition has no unit form.

use super::tests::{export, has_unit, unit, warned};

#[test]
fn depends_on_a_service_orders_the_job_after_it() {
	let yaml = "\
services:
  db:
    image: postgres
jobs:
  migrate:
    image: alpine
    depends_on:
      db:
        condition: service_started
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	let container = unit(&out, "proj-migrate.container");
	assert!(container.contains("After=proj-db.service\n"), "{container}");
	assert!(
		container.contains("Requires=proj-db.service\n"),
		"{container}"
	);
}

#[test]
fn depends_on_a_job_is_ordered_only_when_that_job_has_a_unit() {
	let yaml = "\
jobs:
  prepare:
    image: alpine
    triggers:
      schedule: ['0 2 * * *']
  manual_only:
    image: alpine
    triggers: {manual: true}
  main:
    image: alpine
    depends_on:
      prepare: {condition: service_started}
      manual_only: {condition: service_started, required: false}
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	let container = unit(&out, "proj-main.container");
	assert!(
		container.contains("After=proj-prepare.service\n"),
		"{container}"
	);
	assert!(!container.contains("manual_only"), "{container}");
	assert!(warned(
		&out,
		"job 'main': `depends_on` job 'manual_only' is not ordered"
	));
}

#[test]
fn a_job_inside_a_pod_project_is_not_exported() {
	let yaml = "\
x-podman-pod: true
services:
  web:
    image: nginx
jobs:
  j:
    image: alpine
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	assert!(!has_unit(&out, "proj-j.container"));
	assert!(!has_unit(&out, "proj-j-1.timer"));
	assert!(warned(
		&out,
		"job 'j': schedule entry 1 was not exported: with `x-podman-pod: true`"
	));
}

#[test]
fn a_job_without_an_image_to_run_is_not_exported() {
	let yaml = "\
jobs:
  j:
    build:
      context: .
      dockerfile_inline: 'FROM alpine'
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	assert!(!has_unit(&out, "proj-j.container"));
	assert!(warned(
		&out,
		"job 'j': schedule entry 1 was not exported: the job has no image"
	));
}

#[test]
fn a_job_with_a_build_gets_a_build_unit_that_the_container_uses() {
	let yaml = "\
jobs:
  j:
    build: ./job
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	assert!(has_unit(&out, "proj-j.build"));
	assert!(unit(&out, "proj-j.container").contains("Image=proj-j.build\n"));
}

const PREPARE_THEN_PUBLISH: &str = "\
jobs:
  prepare:
    image: alpine
    triggers: {manual: true}
  publish:
    image: alpine
    depends_on:
      prepare:
        condition: service_completed_successfully
        required: REQUIRED
    triggers:
      schedule: ['0 3 * * *', '0 4 * * *']
";

#[test]
fn a_required_dependency_on_a_job_without_a_unit_blocks_the_dependent() {
	let yaml = PREPARE_THEN_PUBLISH.replace("REQUIRED", "true");
	let out = export(&yaml);
	assert!(!has_unit(&out, "proj-publish.container"));
	assert!(!has_unit(&out, "proj-publish-1.timer"));
	assert!(!has_unit(&out, "proj-publish-2.timer"));
	assert!(warned(
		&out,
		"job 'publish': schedule entry 2 was not exported: it requires job 'prepare'"
	));
}

#[test]
fn a_dependency_is_required_unless_it_says_otherwise() {
	let short = "\
jobs:
  prepare:
    image: alpine
    triggers: {manual: true}
  publish:
    image: alpine
    depends_on: [prepare]
    triggers:
      schedule: ['0 3 * * *']
";
	assert!(!has_unit(&export(short), "proj-publish.container"));
	let implicit = PREPARE_THEN_PUBLISH.replace("        required: REQUIRED\n", "");
	assert!(!has_unit(&export(&implicit), "proj-publish.container"));
}

#[test]
fn an_optional_dependency_on_a_job_without_a_unit_is_dropped_with_a_warning() {
	let yaml = PREPARE_THEN_PUBLISH.replace("REQUIRED", "false");
	let out = export(&yaml);
	let container = unit(&out, "proj-publish.container");
	assert!(!container.contains("prepare"), "{container}");
	assert!(warned(&out, "`depends_on` job 'prepare' is not ordered"));
}

#[test]
fn a_block_travels_up_a_chain_of_required_dependencies() {
	let yaml = "\
jobs:
  c:
    image: alpine
    triggers: {manual: true}
  b:
    image: alpine
    depends_on: [c]
    triggers:
      schedule: ['0 2 * * *']
  a:
    image: alpine
    depends_on: [b]
    triggers:
      schedule: ['0 3 * * *']
";
	let out = export(yaml);
	assert!(!has_unit(&out, "proj-a.container"));
	assert!(!has_unit(&out, "proj-b.container"));
	assert!(warned(
		&out,
		"job 'a': schedule entry 1 was not exported: it requires job 'b'"
	));
}

fn dst_warnings(out: &super::super::QuadletOutput) -> usize {
	out.warnings
		.iter()
		.filter(|w| w.contains("daylight saving"))
		.count()
}

#[test]
fn a_zone_with_summer_time_or_the_local_zone_gets_one_warning_per_job() {
	let yaml = "\
jobs:
  local:
    image: alpine
    triggers:
      schedule: ['0 3 * * *', '0 4 * * *']
  madrid:
    image: alpine
    triggers:
      schedule:
        - {cron: '30 2 * * *', timezone: Europe/Madrid}
        - {cron: '30 3 * * *', timezone: America/Bogota}
";
	let out = export(yaml);
	assert_eq!(dst_warnings(&out), 2, "{:?}", out.warnings);
	assert!(warned(
		&out,
		"job 'local': systemd does not treat daylight saving"
	));
	assert!(warned(
		&out,
		"job 'madrid': systemd does not treat daylight saving"
	));
}

#[test]
fn utc_and_fixed_offset_zones_get_no_daylight_saving_warning() {
	let yaml = "\
jobs:
  utc:
    image: alpine
    triggers:
      schedule:
        - {cron: '0 3 * * *', timezone: UTC}
        - {cron: '0 4 * * *', timezone: GMT}
        - {cron: '0 5 * * *', timezone: Etc/GMT+5}
";
	let out = export(yaml);
	assert_eq!(dst_warnings(&out), 0, "{:?}", out.warnings);
}

#[test]
fn a_container_name_shared_with_a_service_blocks_the_job() {
	let yaml = "\
services:
  web:
    image: nginx
    container_name: shared
jobs:
  bk:
    image: alpine
    container_name: shared
    triggers:
      schedule: ['0 3 * * *', '0 4 * * *']
";
	let out = export(yaml);
	assert!(!has_unit(&out, "proj-bk.container"));
	assert!(!has_unit(&out, "proj-bk-1.timer"));
	assert!(warned(
		&out,
		"job 'bk': schedule entry 2 was not exported: its container name 'shared' is also used by proj-web.container"
	));
}

#[test]
fn two_jobs_with_the_same_container_name_do_not_both_export() {
	let yaml = "\
jobs:
  a:
    image: alpine
    container_name: same
    triggers:
      schedule: ['0 3 * * *']
  b:
    image: alpine
    container_name: same
    triggers:
      schedule: ['0 4 * * *']
";
	let out = export(yaml);
	assert!(has_unit(&out, "proj-a.container"));
	assert!(!has_unit(&out, "proj-b.container"));
	assert!(warned(&out, "also used by proj-a.container"));
}

#[test]
fn a_schedule_that_can_never_fire_is_exported_with_a_warning() {
	let yaml = "\
jobs:
  j:
    image: alpine
    triggers:
      schedule: ['0 0 30 2 *', '0 0 29 2 *']
";
	let out = export(yaml);
	assert!(has_unit(&out, "proj-j-1.timer"));
	assert!(warned(
		&out,
		"job 'j': schedule entry 1 is exported but never fires"
	));
	assert!(!warned(
		&out,
		"schedule entry 2 is exported but never fires"
	));
}

#[test]
fn every_timer_asks_for_one_second_accuracy() {
	let yaml = "jobs:\n  j:\n    image: alpine\n    triggers:\n      schedule: ['0 3 * * *']\n";
	let out = export(yaml);
	assert!(unit(&out, "proj-j-1.timer").contains("AccuracySec=1s\n"));
}
