# Development and release

## Layout

| Path | Contents |
| --- | --- |
| `src/main.rs` | Process entry point and the `open`, `picker`, and `record-focus` flows |
| `src/app.rs` | Picker query, visible rows, selection, and warning state |
| `src/herdr.rs` | Herdr command execution and snapshot-to-workspace conversion |
| `src/ui.rs` | Terminal input, popup rendering, selection, and error display |
| `src/candidate.rs` | Candidate merging, path normalization, deduplication, and ordering |
| `src/fuzzy.rs` | Fuzzy matching that preserves candidate order |
| `src/mru.rs` | Locked and atomic MRU state persistence |
| `src/zoxide.rs` | Optional zoxide query and record parsing |
| `src/lib.rs` | Library exports used by the binary and integration tests |
| `tests/` | Tests of the public library boundary and of the process contract through a pseudo-terminal |
| `herdr-plugin.toml` | Plugin actions, popup pane, focus event, and install build step |
| `docs/demo/` | VHS tape and start script for the recorded README demo |
| `scripts/fetch-or-build.sh` | Verified release download with a Cargo source-build fallback |
| `scripts/check-version.py` | Cargo, plugin manifest, and release-tag version agreement |
| `scripts/release.py` | Release metadata, verification, commit, tag, and optional push |
| `scripts/release-notes.py` | Changelog section extraction for GitHub release notes |
| `.github/workflows/ci.yml` | Pull request and `main` verification gate |
| `.github/workflows/release.yml` | Tagged multi-platform, GitHub, and crates.io publishing |

## Runtime behavior

The plugin has three process entry points:

1. `open` asks Herdr to open the plugin's `picker` popup.
2. `picker` reads a Herdr snapshot and MRU state, then shows the open workspaces. A background
   thread loads zoxide records and checks their directories on worker threads. The picker merges
   those directories into the list while it waits for input, then focuses or creates the selected
   workspace.
3. `record-focus` handles Herdr's `workspace.focused` event and records that workspace's canonical
   directory in MRU state.

Candidate source order has one invariant: open workspaces come first, then remaining zoxide entries.
Within each group, stored MRU paths come first. Remaining workspaces use Herdr's order and remaining
zoxide entries use descending score. A non-empty query ranks every matching candidate by fuzzy
relevance and uses source order to break score ties. Canonical paths deduplicate the two sources.

A directory selection reads a second Herdr snapshot before creating anything. If another process
opened the directory after the picker loaded, the plugin focuses that workspace instead of creating
a duplicate.

MRU state lives in the directory Herdr supplies through `HERDR_PLUGIN_STATE_DIR`. Writes use a lock
file and a temporary file followed by rename. The store retains at most 200 paths. Invalid JSON is
moved to a uniquely named backup and treated as empty state.

## Commands

| Command | Purpose |
| --- | --- |
| `just format` | Format all Rust targets |
| `just format-check` | Check formatting without changing files |
| `just clippy` | Run Clippy on every workspace target with warnings denied |
| `just cargo-check` | Type-check the locked workspace |
| `just build` | Build the locked workspace |
| `just test` | Run unit and integration tests |
| `just test-integration` | Run only integration test targets |
| `just deny` | Check advisories, licenses, sources, and banned dependencies |
| `just machete` | Find unused dependencies |
| `just check` | Run every static, build, and dependency check |
| `just verify` | Run `just check` and the full test suite |
| `just demo` | Record the README demo with VHS |

CI installs the pinned command versions listed in `CONTRIBUTING.md` and runs `just verify`. Run the
same command locally so CI does not apply a second convention.

## Testing

Unit tests live beside the module they exercise. `tests/cli.rs` runs the built binary against fake
`herdr` and `zoxide` scripts. Picker tests open a pseudo-terminal, wait for expected output, then
send keys, so they also check raw mode and terminal restoration. They cover candidate precedence and
deduplication, fuzzy-order stability, snapshot conversion, MRU recovery and concurrent writes,
terminal rendering, and zoxide parsing. `tests/core_api.rs` protects the public library boundary
used by the binary.

Tests must not depend on a live Herdr server, a user's zoxide database, or fixed home-directory
contents. The CLI test fixture sets `HOME` and `HERDR_WORKSPACER_ZOXIDE_PATH` to its own directory
and installs a fake zoxide by default. Build values explicitly and use isolated temporary
directories for filesystem behavior. Do not wait with a fixed delay. When a test needs a fake
binary to wait, make the fake wait for a file that the test creates. Prefer an assertion on the
result or state transition over an assertion on private call order.

For a behavior change:

1. Add a regression test that fails for the incorrect observable result.
2. Make the smallest source change that restores the invariant.
3. Run the narrow test while iterating.
4. Run `just verify` before committing.

## Testing with Herdr

Build and link the working tree:

```sh
cargo build
mkdir -p bin
cp target/debug/herdr-workspacer bin/herdr-workspacer
herdr plugin link .
herdr plugin action invoke herdr-workspacer.open
```

Test with zoxide available and unavailable. Confirm that open workspaces remain selectable in both
cases. Select an existing workspace, a zoxide-only directory, and a directory that becomes a
workspace while the popup is open. Restart Herdr and confirm that recorded paths retain MRU order.

The plugin manifest runs `bin/herdr-workspacer` relative to the plugin directory, which Herdr uses
as the working directory. Recopy the debug binary after rebuilding, or
relink the plugin if the Herdr development workflow replaces the plugin directory.

## Demo recording

`README.md` shows a recorded picker session. Record it with
[VHS](https://github.com/charmbracelet/vhs):

```sh
herdr plugin link .
just demo
```

The tape starts `docs/demo/start-herdr-workspacer-demo.sh`, which builds the working tree into
`bin/`, writes demo directories and a demo zoxide database under a temporary `HOME`, and opens a
separate Herdr session named `workspacer-demo`. The recording shows those demo paths instead of a
developer's own directories. Link the plugin first, or the recording uses the installed release.

MRU state stays in the running Herdr installation, so the demo directories remain in local MRU
state. `HERDR_WORKSPACER_DEMO_HOME` and `HERDR_WORKSPACER_DEMO_SESSION` change the temporary home
and the session name.

## Continuous integration

`.github/workflows/ci.yml` runs on pull requests and pushes to `main` with read-only repository
permissions. Its quality job runs the complete Rust gate on Ubuntu. A separate metadata job checks
that `Cargo.toml` and `herdr-plugin.toml` carry the same version.

Dependabot proposes monthly updates for Cargo and GitHub Actions. Dependency updates must pass the
same lint, test, license, advisory, source, and unused-dependency checks as source changes.

## Release

A tag named `v<version>` starts `.github/workflows/release.yml`. The workflow checks that the tag,
`Cargo.toml`, and `herdr-plugin.toml` agree, then builds four assets:

- Linux x86-64
- Linux ARM64
- macOS x86-64
- macOS ARM64

Linux assets use `cross`; macOS assets use Cargo on the matching runner. The GitHub publish job
creates `SHA256SUMS`, attaches all assets, and uses the matching `CHANGELOG.md` section as the
release body. The final job publishes the same version to crates.io unless that version already
exists.

Prepare a release from a clean `main`:

```sh
just release <version>
```

The command requires a three-part version and refuses dirty worktrees, other branches, existing
tags, decreasing versions, and empty `Unreleased` changelog sections. It then:

1. Sets the version in `Cargo.toml` and `herdr-plugin.toml`.
2. Adds a dated changelog section while retaining an empty `Unreleased` section.
3. Updates `Cargo.lock` and runs `just verify`. It restores the release files if verification fails.
4. Commits `chore: release <version>` and creates `v<version>`.

Pushing stays separate because it makes the release public:

```sh
git push origin main
git push origin v<version>
```

Pass `--push` to perform both pushes after the commit and tag are created:

```sh
just release <version> --push
```

Before the first release, publish the crate manually because crates.io cannot configure trusted
publishing for a package that does not exist:

```sh
just release 0.1.0
git push origin main
cargo login
cargo publish --locked
git push origin v0.1.0
```

After the first publish, add a crates.io trusted publisher for owner `mcuste`, repository
`herdr-workspacer`, workflow `release.yml`, and environment `crates-io`. Later tag workflows use a
short-lived OIDC credential and need no stored crates.io token.

Review the GitHub assets, checksums, and crates.io version after the workflow completes. The install
script uses the version in `Cargo.toml` to select the GitHub release and refuses a binary whose
checksum does not match.
