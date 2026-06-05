# podman-compose

`lynx-compose` — docker-compose translator and runner for rootless Podman,
built in Rust.

It parses `docker-compose.yml` files and drives rootless Podman to create the
equivalent containers, networks and volumes. Used by the
[Lynx panel](https://github.com/Glyndor/panel) through the
[panel-agent](https://github.com/Glyndor/panel-agent), and usable standalone
as a CLI and as a library (`lynx_compose`).

## Build

```bash
cargo build --release
cargo test
```

## Usage

```bash
lynx-compose up -f docker-compose.yml
```

## Contributing & security

See the org-wide [contributing guide](https://github.com/Glyndor/.github/blob/main/CONTRIBUTING.md).
Report vulnerabilities privately via the Security tab — never in a public issue.

## License

[Apache-2.0](LICENSE)
