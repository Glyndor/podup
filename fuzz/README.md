# Fuzzing

Coverage-guided fuzz targets for podup's untrusted-input surfaces, built on
[`cargo-fuzz`](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer).

## Targets

| Target | Surface |
|--------|---------|
| `parse_compose` | Full compose parse: substitution, YAML, anchor/alias merge keys, type coercion (`parse_str` / `parse_str_raw`) |
| `substitute` | Variable substitution and its modifiers (`${VAR:-default}`, `${VAR:?err}`, escapes) |
| `size` | Memory / CPU / duration parsers (f64 → integer casts) |
| `dotenv` | dotenv quote/escape/comment/multi-line handling |
| `stream_frame` | libpod multiplexed frame header and JSON-line splitter over raw daemon bytes |
| `stream_ndjson` | libpod newline-delimited JSON stream decoding |
| `extract_tar` | container to host tar extraction (`cp`) on hostile archives |
| `ports` | Short and long port forms: no panic, accepted ports re-serialise and re-parse to themselves |
| `duration` | Pull-policy intervals against a wide-integer reference; float parsers never return a saturated cast |
| `ignore_glob` | `.dockerignore` matcher: no panic, bounded time, equal to a plain recursive matcher on short inputs |
| `cron_calendar` | crontab to `OnCalendar=`: no panic, deterministic, safe characters, same times as the cron line |
| `audit_bind_path` | Host path folding behind `audit`'s sensitive bind check: idempotent, folding only adds findings |
| `sanitize` | Terminal escaping of names and table cells: no control character survives, safe text unchanged |
| `libpod_json` | Every daemon response type decoded from arbitrary JSON, deep nesting and huge numbers included |
| `parse_version` | Release version parser behind the self-update comparison: no panic on a malformed tag |
| `expected_digest` | `SHA256SUMS` manifest lookup: no panic, and the digest handed back is 64 hex characters |
| `parse_latest_tag` | Release-metadata parser: no panic on a body that is not the expected JSON |

`audit_bind_path` compiles `internal/command_line/audit/check_sensitive_bind.rs` into the fuzz crate
(`fuzz/support/audit`), because the audit lives in the `podup` binary and a fuzz crate cannot link a binary.

The `dotenv` and `stream_frame` targets reach crate-private parsers through the
`test-helpers` feature (`podup::fuzz_api`), which is off by default and not part
of the published API.

## Running

Requires a nightly toolchain and `cargo-fuzz`:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz

# Run one target (Ctrl-C to stop):
cargo +nightly fuzz run parse_compose

# Time-boxed (CI-style):
cargo +nightly fuzz run parse_compose -- -max_total_time=60 -rss_limit_mb=2048
```

Crashing inputs are written to `fuzz/artifacts/<target>/`; reproduce with
`cargo +nightly fuzz run <target> <artifact-path>`.
