# Changelog

All notable changes to this project are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- The picker opens with the open workspaces at once and adds zoxide directories when they load.
- zoxide directory checks run in parallel, so one unresponsive path no longer delays the popup.
- `merge_candidates` no longer checks that zoxide paths exist. Use `load_zoxide_directories` to
  get checked entries.
- Terminal setup uses termios calls instead of running `stty`.
- Plugin commands run the binary directly instead of through `sh`.
- zoxide discovery skips install locations that do not exist.

## [0.2.0] - 2026-08-31

### Added

- The picker shows linked Git worktrees with a distinct tag and label.

### Fixed

- Primary Git workspaces no longer appear as linked worktrees.

## [0.1.2] - 2026-08-26

### Changed

- Refreshed the README demo with clearer picker framing, reliable action invocation, user configuration, and isolated gcloud state.

## [0.1.1] - 2026-08-22

### Fixed

- GitHub release publishing now identifies the repository without a checkout.

## [0.1.0] - 2026-08-22

### Added

- Herdr popup workspace picker with MRU ordering and fuzzy filtering.
- Open-workspace and optional zoxide candidate sources.
- Path-based MRU state with atomic locked updates.
- Release-binary installation with SHA-256 verification and Cargo fallback.
