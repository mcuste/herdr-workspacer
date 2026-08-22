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
| `tests/` | Tests of the public library boundary |
| `herdr-plugin.toml` | Plugin actions, popup pane, focus event, and install build step |
| `scripts/fetch-or-build.sh` | Verified release download with a Cargo source-build fallback |
| `scripts/check-version.py` | Cargo, plugin manifest, and release-tag version agreement |
| `.github/workflows/ci.yml` | Pull request and `main` verification gate |
| `.github/workflows/release.yml` | Tagged multi-platform builds and GitHub release publishing |

## Runtime behavior

The plugin has three process entry points:

1. `open` asks Herdr to open the plugin's `picker` popup.
2. `picker` reads a Herdr snapshot, MRU state, and optional zoxide records. It merges and displays
   candidates, then focuses or creates the selected workspace.
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

CI installs the pinned command versions listed in `CONTRIBUTING.md` and runs `just verify`. Run the
same command locally so CI does not apply a second convention.

## Testing

Unit tests live beside the module they exercise. They cover candidate precedence and deduplication,
fuzzy-order stability, snapshot conversion, MRU recovery and concurrent writes, terminal rendering,
and zoxide parsing. `tests/core_api.rs` protects the public library boundary used by the binary.

Tests must not depend on a live Herdr server, a user's zoxide database, or fixed home-directory
contents. Build values explicitly and use isolated temporary directories for filesystem behavior.
Prefer an assertion on the result or state transition over an assertion on private call order.

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

The plugin manifest invokes `bin/herdr-workspacer`. Recopy the debug binary after rebuilding, or
relink the plugin if the Herdr development workflow replaces the plugin directory.

## Continuous integration

`.github/workflows/ci.yml` runs on pull requests and pushes to `main` with read-only repository
permissions. Its quality job runs the complete Rust gate on Ubuntu. A separate metadata job checks
that `Cargo.toml` and `herdr-plugin.toml` carry the same version.

Dependabot proposes monthly updates for Cargo and GitHub Actions. Dependency updates must pass the
same lint, test, license, advisory, source, and unused-dependency checks as source changes.

## Release

A tag named `v<version>` starts `.github/workflows/release.yml`. The workflow first checks that the
tag, `Cargo.toml`, and `herdr-plugin.toml` agree. It then builds four assets:

- Linux x86-64
- Linux ARM64
- macOS x86-64
- macOS ARM64

Linux assets use `cross`; macOS assets use Cargo on the matching runner. The publish job creates
`SHA256SUMS` from the downloaded build artifacts and attaches all assets to a GitHub release.

To prepare a release from a clean `main`:

1. Replace `## [Unreleased]` in `CHANGELOG.md` with `## [<version>] - YYYY-MM-DD` and add a new
   empty `Unreleased` section above it.
2. Set the same version in `Cargo.toml` and `herdr-plugin.toml`.
3. Run `cargo check` once to update the root package entry in `Cargo.lock`.
4. Run `python3 scripts/check-version.py` and `just verify`.
5. Commit with `chore: release <version>` and create the tag `v<version>` on that commit.
6. Push the commit, then push the tag.

The tag makes the release public. Review the asset names and checksums after the workflow completes.
The install script uses the version in `Cargo.toml` to select that release and refuses a binary whose
checksum does not match.
