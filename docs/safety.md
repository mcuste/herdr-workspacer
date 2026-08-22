# Safety model

Herdr Workspacer joins data from a Herdr session, zoxide output, filesystem paths, and a local state
file. Each source can contain stale or malformed values. The plugin validates structure at the
boundary and keeps optional-source failures from removing valid Herdr workspaces.

## Trust boundaries

| Input | Treatment |
| --- | --- |
| Herdr executable path | Read only from `HERDR_BIN_PATH`, which the Herdr plugin runtime supplies |
| Herdr snapshot | Parsed as JSON into the fields needed to resolve workspace directories |
| Focus event | Parsed as JSON and reduced to a workspace ID present in a fresh snapshot |
| zoxide output | Parsed one record at a time; invalid scores, empty paths, and missing directories are skipped |
| MRU state | Parsed as versioned JSON; malformed state is backed up and replaced with empty history |
| Terminal labels and paths | Control characters are replaced before display |
| Selected paths | Passed to Herdr as one process argument, never interpolated into a command string |

The plugin trusts Herdr to set its runtime environment correctly. Anyone who can replace the binary
named by `HERDR_BIN_PATH`, change the `zoxide` executable found on `PATH`, or write the plugin state
directory already controls the plugin process. The plugin does not try to create a sandbox inside
that boundary.

## External commands

Rust starts Herdr and zoxide through `std::process::Command` with argument arrays. Candidate labels,
workspace IDs, and directory paths do not pass through a shell. Standard input is closed for Herdr
commands so a background plugin action cannot wait for terminal input.

The plugin runs these external operations:

| Operation | Inputs | Result |
| --- | --- | --- |
| `herdr api snapshot` | Fixed arguments | JSON session snapshot |
| `herdr plugin pane open` | Fixed plugin and pane IDs | Opens the picker popup |
| `herdr workspace focus` | Workspace ID from the snapshot | Focuses one open workspace |
| `herdr workspace create` | Selected path as a distinct argument | Creates and focuses a workspace |
| `zoxide query -ls` | Fixed arguments | Optional scored directory records |

A non-zero Herdr exit is an error. Its trimmed standard error is shown to the user. A zoxide failure
is non-fatal because zoxide is an optional candidate source.

The manifest uses `sh -c` only to locate the installed binary through the fixed
`HERDR_PLUGIN_ROOT` environment variable and replace the shell with that binary. No candidate or
session value is inserted into those command strings.

## Path handling

Open-workspace and zoxide paths are converted to absolute paths and canonicalized when possible.
Canonical paths provide identity for deduplication and MRU ranking. The original path remains the
one sent to Herdr when creating a workspace, so symlink-based directory choices keep their user
visible spelling.

Only zoxide paths that are directories at picker load time become candidates. A selection is not a
filesystem authorization check: Herdr decides whether the current user may create a workspace at
that path. The plugin performs no file reads inside candidate directories.

Workspace labels and displayed paths replace terminal control characters. Search text retains the
original data for matching but never reaches terminal rendering directly.

## MRU state

Herdr chooses the state directory and passes it as `HERDR_PLUGIN_STATE_DIR`. The plugin writes only
these entries there:

- `mru.json` for at most 200 canonical paths
- `mru.lock` while reading or updating state
- `.mru.json.<unique>.tmp` during an atomic update
- `mru.json.invalid-<unique>` when existing state is malformed

Updates take an exclusive create-only lock, write and sync a new file, then rename it over the state
file. A lock older than five seconds is treated as stale. Concurrent focus events therefore merge
through the latest state instead of replacing one another with independently loaded copies.

A missing state file and malformed JSON both produce empty history. Other filesystem errors remain
visible because silently discarding a permission or storage failure would make ordering unreliable.

## Installation and releases

The install build step first tries the GitHub release asset for the exact package version and
platform. It downloads both the binary and `SHA256SUMS`, calculates the asset digest locally, and
installs the binary only when they match. The temporary download directory is removed on exit.

The checksum detects incomplete or mismatched downloads. Because the asset and checksum come from
the same GitHub release, it does not protect against a compromised repository or release publisher.
Release workflow permissions and GitHub account security remain part of the trust model.

If a verified release is unavailable, the installer builds the committed source with
`cargo build --release --locked`. It never runs an unverified downloaded binary. If neither path is
available, installation fails with an explanation.

The release workflow validates version agreement before building. Every release asset comes from
the tagged source, and the workflow creates checksums only after collecting all platform artifacts.

## Reporting a vulnerability

Report a vulnerability through the repository's
[private security advisory form](https://github.com/mcuste/herdr-workspacer/security/advisories/new).
Include the affected version, platform, reproduction steps, and the boundary that was crossed. Do
not open a public issue until a fix is available.
