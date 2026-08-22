# Herdr Workspacer

MRU fuzzy workspace picker for Herdr, powered by open workspaces and zoxide.

## Status

Repository setup is complete. The picker implementation has not started.

## Planned support

- Herdr 0.8 or later
- macOS and Linux
- Optional zoxide integration

## Development

Requires the Rust stable toolchain, just 1.58.0, cargo-deny 0.20.2, and cargo-machete 0.9.2.

```sh
cargo install just --version 1.58.0 --locked
cargo install cargo-deny --version 0.20.2 --locked
cargo install cargo-machete --version 0.9.2 --locked
just --list
just verify
```

## License

MIT. See [LICENSE](LICENSE).
