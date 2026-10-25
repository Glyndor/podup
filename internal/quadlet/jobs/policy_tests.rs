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
    depends_on: [prepare, manual_only]
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
