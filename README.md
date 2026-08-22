# Herdr Workspacer

MRU fuzzy workspace picker for Herdr, powered by open workspaces and zoxide.

## Status

Repository setup is complete. The picker implementation has not started.

## Planned support

- Herdr 0.8 or later
- macOS and Linux
- Optional zoxide integration

## Development

Requires the Rust stable toolchain, cargo-deny 0.20.2, and cargo-machete 0.9.2.

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo install cargo-machete --version 0.9.2 --locked
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo deny check
cargo machete
```

## License

MIT. See [LICENSE](LICENSE).
