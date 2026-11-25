#cloud-config
growpart: {mode: auto, devices: ['/']}
resize_rootfs: true
users:
  - name: tester
    sudo: 'ALL=(ALL) NOPASSWD:ALL'
    shell: /bin/bash
write_files:
  # Podman's default log driver on Fedora is journald, and in this image
  # every logs request comes back `500 ... unable to open a handle to the
  # library`, because its go-systemd binding cannot open the journal. Installing
  # systemd-devel does not help (measured: the package installs and the
  # error persists), so rather than keep guessing at the dlopen, use the
  # file-based driver, which has no library to open.
  #
  # This is not cosmetic. With logs unavailable the suite's five logs
  # tests passed without fetching a byte: `logs` swallowed the 500 and
  # returned success, so their `unwrap()` succeeded on nothing.
  #
  # The events backend reads the same journal through the same binding,
  # and was left on journald: `niche::engine_events_stream_connects`
  # failed with `Api { status: 500, message: "unable to open a handle
  # to the library" }`, the logs error word for word. `events_logger =
  # "file"` takes the journal out of that path too. Not measured again
  # on 6.2: the lane cannot show it
  # without a run that drops the line.
  - path: /etc/containers/containers.conf
    content: |
      [containers]
      log_driver = "k8s-file"
      [engine]
      events_logger = "file"
  - path: /usr/local/bin/run-suite.sh
    permissions: '0755'
    content: |
      #!/bin/bash
      export XDG_RUNTIME_DIR=/run/user/$(id -u)
      systemctl --user start podman.socket
      export PODMAN_SOCKET=$XDG_RUNTIME_DIR/podman/podman.sock
      # In here Podman is guaranteed, so a test that cannot reach it
      # must fail rather than skip: a skipped test reports `ok`, and a
      # VM whose socket never came up would otherwise look like a
      # flawless 155-pass run and clear the floor comfortably.
      export PODUP_REQUIRE_PODMAN=1
      # Same for the Quadlet generator that ships with this Podman: the
      # tests that run it skip on a runner whose Podman is older than the
      # floor (the ubuntu-latest 4.9 does not read the keys the exporter
      # writes), and here a skip would report `ok` for a round-trip that
      # never ran. With this set, a missing or older generator fails them.
      export PODUP_REQUIRE_QUADLET_GENERATOR=1
      cd "$HOME/podup" || exit 90
      # Coverage runs on the scheduled and manual paths only, never on a
      # pull request. The number is worth having where the integration
      # tests can actually run (#1326: the CI coverage job sees 79% because
      # every integration test skips itself without Podman, while the full
      # suite covers 91.5%), but instrumenting doubles nothing here for
      # free: it is a second full build inside the VM. Gating on it is
      # deliberately NOT done yet: a threshold set from a number never
      # observed in this environment is how a phantom check gets born.
      #
      # The suite and the coverage are two runs of two machines now, each with its own
      # budget (.github/podman_lane/budget): MODE says which one this VM is. In `suite` it
      # runs the gating suite, the leak scan and the generator tests, and measures no
      # coverage; in `coverage` it runs the instrumented suite and nothing else. The first
      # version ran both in one boot and the sum never fitted in the 70 minutes the VM gets.
      MODE=__MODE__
      # PODUP_PHASE lines say how far the VM got and when, by its own clock, so that a cut
      # at the budget names the phase it was in and a measurement of the next budget has
      # every number it needs.
      phase() { echo "PODUP_PHASE_BEGIN name=$1 uptime=$(cut -d. -f1 /proc/uptime) PODUP_PHASE_END"; }
      phase setup-done
      # Capture the FULL output: the per-failure detail (where the flake
      # marker lives) precedes the summary, so a `tail` on the console
      # would truncate it and hide why a test failed.
      # Retry once on a nested-virt transient (a dropped connection). A real
      # failure reproduces on the second run, so this stabilises the lane for
      # required-check gating without masking genuine bugs.
      #
      # WHAT COUNTS AS A DROP, defined once, used by both the retry above and
      # the FLAKY counter below, because the two drifting apart is how this
      # went wrong (#1104). The list used to hold only the head-level shapes,
      # and a dropped BODY is none of them: measured against a socket that
      # severs a chunked body on purpose, hyper reports a Body error wrapping
      # io UnexpectedEof, worded "unexpected EOF during chunk size line" when
      # the cut lands between chunks and IncompleteBody when it lands
      # mid-payload, with the Display form "error reading a body from
      # connection". None matched, so a drop mid-body was neither retried nor
      # counted; it just reddened the leg, which is exactly what happened to
      # fifteen streaming tests when a mid-stream error was made fatal.
      #
      # EXPERIMENT (#1039): the suite and the coverage run both use one
      # test thread.
      #
      # Every hypothesis for Podman 6's failing tail has been falsified in
      # turn: journald, SELinux, and a boot race on systemd-resolved, the
      # last one killed by its own instrumentation. What survives is the
      # shape of the error: the path that is missing is the mount
      # DESTINATION inside Podman's rootless-netns tree, not a host file,
      # which reads as the shared netns being torn down under a concurrent
      # container creation. One thread is the cheapest way to test that: if
      # the signature disappears, concurrency is the lever; if it survives,
      # this hypothesis dies too and the search moves elsewhere.
      #
      # `--test-threads` caps how many streaming tests hit the libpod
      # socket at once.
      #
      # This used to say the drops were specific to the nested-virt
      # (VM-in-runner) transport, and that a single-level VM "never
      # drops a byte". **That is measured false** (2026-08-03, #1322):
      # on a plain KVM guest the same suite fails 17 of 27 at eight
      # test threads and 3 at one, a clean dose-response curve in
      # thread count, reproducible across repetitions. Nesting makes
      # it worse; it does not cause it. (Measured on Podman 6.0.2;
      # not measured again on 6.2.)
      #
      # What holds is the lever: fewer concurrent streams means less
      # peak pressure,
      # which is the lever to shrink Podman 6's variable flaky tail toward
      # a small, stable set that an identity list can gate (#1039). Kept at
      # 2 rather than 1 so the suite still fits the VM's time budget.
      if [ "$MODE" = suite ]; then
      DROPPED='IncompleteMessage|connection closed before message completed|channel closed|Connection reset|error reading a body from connection|unexpected EOF during chunk size line|IncompleteBody|body-unexpected-eof'
      for attempt in 1 2; do
        RUST_LOG=podup=warn cargo test --features test-helpers --test engine_integration -- --test-threads=1 > /tmp/cargo.log 2>&1
        RC=$?
        [ "$RC" -eq 0 ] && break
        grep -qE "$DROPPED" /tmp/cargo.log || break
        echo "=== nested-virt transient on attempt $attempt; retrying suite ==="
      done
      # Leak scan, run in a separate cargo process after engine_integration
      # has exited. A second `cargo test --test` invocation cannot overlap
      # the first, so the scanner genuinely runs after the suite. A red
      # suite still scans: chaining with && would skip the scan exactly
      # when we most want to see what was left on the host.
      #
      # The PID comes from target/podup-leak-scan-pid, which
      # engine_integration writes on the first call to proj(). When
      # the suite retries, the file holds the last attempt's PID, so
      # only the last attempt is scanned.
      #
      # A missing file is reported as `rc=nopid`, not scanned as
      # nothing: the scan test exits 0 when it is given no PID, and a
      # suite that died before writing the file would otherwise pass
      # this gate having scanned nothing at all.
      #
      # Same `--features` as the suite above, so cargo reuses the
      # build instead of recompiling the crate inside the VM.
      if [ -s target/podup-leak-scan-pid ]; then
        PODUP_LEAK_SCAN_PID="$(cat target/podup-leak-scan-pid)" RUST_LOG=podup=warn \
          cargo test --features test-helpers --test integration_leak_scan > /tmp/leak-scan.log 2>&1
        SCAN_RC=$?
        if [ "$SCAN_RC" -ne 0 ]; then
          echo "=== leak scan failures ==="
          tail -60 /tmp/leak-scan.log
          sed -n '/^failures:$/,/^test result:/p' /tmp/leak-scan.log | head -120
        fi
      else
        SCAN_RC=nopid
      fi
      echo "PODUP_LEAK_SCAN_BEGIN rc=$SCAN_RC PODUP_LEAK_SCAN_END"
      # The tests that run the real Quadlet generator, which no other leg
      # can run: the library's `quadlet::` tests and the hostile binary that
      # round-trips bind, command and env_file values. Both build with the
      # same `--features` as above so cargo reuses the dependencies. Their
      # result is its own marker: the summary above counts the engine suite.
      GEN_RC=0
      cargo test --features test-helpers --lib quadlet:: > /tmp/quadlet-generator.log 2>&1 || GEN_RC=$?
      cargo test --features test-helpers --test hostile_quadlet_units >> /tmp/quadlet-generator.log 2>&1 || GEN_RC=$?
      if [ "$GEN_RC" -ne 0 ]; then
        echo "=== quadlet generator test failures ==="
        tail -60 /tmp/quadlet-generator.log
      fi
      echo "PODUP_QUADLET_GENERATOR_BEGIN rc=$GEN_RC PODUP_QUADLET_GENERATOR_END"
      tail -60 /tmp/cargo.log
      echo "=== failures section (full) ==="
      sed -n '/^failures:$/,/^test result:/p' /tmp/cargo.log | head -120
      # Machine-readable summary computed from the FULL log, using the same
      # drop signatures as the retry above; everything else counts as a real
      # failure.
      PASS=$(grep -oE 'test result: .* [0-9]+ passed' /tmp/cargo.log | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' | tail -1)
      FAIL=$(grep -oE '[0-9]+ failed' /tmp/cargo.log | grep -oE '[0-9]+' | tail -1)
      FLAKY=$(grep -cE "$DROPPED" /tmp/cargo.log)
      echo "PODUP_TESTS_RC_BEGIN rc=$RC PODUP_TESTS_RC_END"
      echo "PODUP_SUMMARY_BEGIN pass=${PASS:-0} fail=${FAIL:-0} flaky=${FLAKY:-0} PODUP_SUMMARY_END"
      phase suite-done
      fi
      # Coverage, measured where the integration tests can actually run.
      # Reported, never gated; see the note where COVERAGE is set. A
      # failure here must not redden the leg: this is an observation, and
      # the suite's own result above is what gates.
      if [ "$MODE" = coverage ]; then
        # Nothing here redirects to /dev/console. This script runs as
        # `tester`, which may not open that device, and a denied
        # redirection aborts the command it was attached to and is
        # reported as that command's own failure. The first run said
        # `install-failed` for exactly that reason: cargo never ran.
        # stdout and stderr already point at the console, inherited
        # from the root caller that launched this script.
        # cargo-llvm-cov looks for the `llvm-tools-preview` rustup
        # component, and there is no rustup here: Rust comes from dnf.
        # It accepts the two tools directly instead, and Fedora's `llvm`
        # package (already installed above) carries both. Taking them
        # from the distribution is also what keeps the versions in step:
        # rustc and llvm are built for the same Fedora release, so the
        # profile format matches, which a separately-pinned LLVM would
        # not guarantee.
        #
        # Exactly ONE PODUP_COVERAGE line is emitted on every path. Two
        # would be worse than none: whoever greps the log gets an answer
        # either way and no reason to suspect there was another.
        export LLVM_COV=/usr/bin/llvm-cov
        export LLVM_PROFDATA=/usr/bin/llvm-profdata
        # Print both versions unconditionally. A profile-format mismatch
        # is the next thing likely to go wrong here, and it is only
        # diagnosable against the pair of numbers that produced it.
        echo "llvm tools: $("$LLVM_COV" --version 2>&1 | grep -i version | head -1)"
        echo "rustc llvm: $(rustc --version --verbose | grep -i '^LLVM')"
        if [ ! -x "$LLVM_COV" ] || [ ! -x "$LLVM_PROFDATA" ]; then
          # Say so here rather than letting cargo-llvm-cov fail with its
          # own wording several minutes and one full build later.
          echo "PODUP_COVERAGE=llvm-tools-missing"
        elif cargo install cargo-llvm-cov --locked --version "=0.9.2"; then
          phase coverage-tool-installed
          # Same thread cap as the gating run above. cargo-llvm-cov
          # drives its own `cargo test` and does NOT inherit it, which
          # is measurable rather than theoretical: on the Podman 6 leg
          # the gating run took 1743s and passed 178/178, and the
          # coverage run of the same code in the same boot took 441s,
          # four times faster because it was parallel, and failed 5
          # with hyper IncompleteMessage. Uncapped, this step measures
          # a different suite than the one that gates.
          cargo llvm-cov --all-features --summary-only \
            -- --test-threads=1 >/tmp/cov.log 2>&1
          phase coverage-run-done
          PCT=$(awk '$1=="TOTAL" { for (i=1;i<=NF;i++) if ($i ~ /%$/) { print $i; exit } }' /tmp/cov.log)
          if [ -n "$PCT" ]; then
            echo "PODUP_COVERAGE=$PCT"
          else
            # Ran, produced nothing parseable. Print what it actually
            # said: a second 15-minute boot is an expensive way to
            # find out, and each distinct marker below names one
            # cause instead of collapsing every failure into one word.
            echo "PODUP_COVERAGE=no-total-line"
            tail -30 /tmp/cov.log
          fi
        else
          echo "PODUP_COVERAGE=install-failed"
        fi
      fi
      if [ "$MODE" = suite ]; then
      # Emit the IDENTITIES of the failing tests, not just the count.
      # The floor gate can only tighten to a per-test known-flaky
      # allowlist once the flaky set is small and stable (#1039), and
      # building that allowlist starts with observing which tests
      # actually fail across nested-virt runs, which the count alone
      # cannot tell you. cargo prints a final `failures:` block of bare
      # test names (the earlier block carries `---- name stdout ----`
      # detail lines, which the 4-space-exact match skips); no failures
      # yields an empty list, not an error.
      FAILED_TESTS=$(sed -n '/^failures:$/,/^test result:/p' /tmp/cargo.log | grep -oE '^    [A-Za-z0-9_:]+$' | tr -d ' ' | paste -sd, -)
      echo "PODUP_FAILED_TESTS_BEGIN tests=${FAILED_TESTS} PODUP_FAILED_TESTS_END"
      fi
runcmd:
  - bash -c 'F=$(findmnt -no FSTYPE /); case $F in btrfs) btrfs filesystem resize max / ;; xfs) xfs_growfs / ;; ext4) resize2fs $(findmnt -no SOURCE /) ;; esac >/dev/console 2>&1; echo "DISK_BEGIN value=$(df -h / | tail -1) DISK_END" >/dev/console'
  - bash -c 'dnf install -y podman rust cargo gcc git __LLVM__ >/dev/console 2>&1'
  # Prove the driver took, so a silently-ignored config cannot put the
  # logs tests back to passing without reading anything.
  - bash -c 'echo "LOGDRIVER_BEGIN value=$(podman info --format {{.Host.LogDriver}} 2>/dev/null) LOGDRIVER_END" >/dev/console'
  - bash -c 'echo "PODMAN_BEGIN version=$(podman --version)  RUST=$(rustc --version) PODMAN_END" >/dev/console'
  - bash -c 'echo "tester:100000:65536" >/etc/subuid; echo "tester:100000:65536" >/etc/subgid'
  - bash -c 'loginctl enable-linger tester; sleep 6'
  - bash -c 'mkdir -p /mnt/repo && mount -t 9p -o ro,trans=virtio,version=9p2000.L repo /mnt/repo && cp -a /mnt/repo /home/tester/podup && chown -R tester:tester /home/tester/podup'
  - bash -c 'sudo -iu tester /usr/local/bin/run-suite.sh >/dev/console 2>&1'
  - bash -c 'echo PROBE_DONE >/dev/console; sleep 2; poweroff'
