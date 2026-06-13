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
