# Contributing

## Before making a change

Keep each change focused on one observable result. Open an issue before a large behavior or
interface change so the design can be settled before implementation. Small fixes and documentation
improvements can go directly to a pull request.

Do not commit local binaries, generated build output, editor state, or planning files.

## Development setup

Install Rust 1.85 or later and the tools used by the verification gate:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo install cargo-machete --version 0.9.2 --locked
cargo install just --version 1.58.0 --locked
```

Run the complete gate before opening or updating a pull request:

```sh
just verify
```

`just verify` checks formatting, Clippy warnings, compilation, release builds, dependency policy,
unused dependencies, unit tests, and integration tests. See [Development and release](docs/development.md)
for the individual commands and manual plugin workflow.

## Tests and changelog

Add or update a test when behavior changes. Test the public result, boundary, or failure mode rather
than private implementation details. Keep tests deterministic and independent of a developer's
Herdr session, home directory, and zoxide database.

Update `CHANGELOG.md` under `Unreleased` for user-visible features, fixes, security changes, and
breaking changes. Tooling-only, test-only, and documentation-only changes do not need an entry.

## Commit convention

Use a concise [Conventional Commits](https://www.conventionalcommits.org/) subject:

```text
<type>(<optional-scope>): <imperative summary>
```

Allowed types:

| Type | Use |
| --- | --- |
| `feat` | New user-visible behavior |
| `fix` | Correct user-visible behavior |
| `perf` | Improve measured performance without changing behavior |
| `refactor` | Change implementation without changing behavior |
| `test` | Add or correct tests only |
| `docs` | Change documentation only |
| `build` | Change build tools, dependencies, or local project gates |
| `ci` | Change continuous integration or release automation |
| `chore` | Repository maintenance that fits no type above |

Subject rules:

- Use lower-case after the colon.
- Start with an imperative verb such as `add`, `fix`, `reject`, or `document`.
- Keep the subject at 72 characters or fewer.
- Do not end the subject with a period.
- Add a scope only when it makes the affected area clearer, such as `mru` or `release`.

Use the body for non-trivial commits. State what changed, why it changed, and any important boundary
or tradeoff. Wrap prose at 72 characters where practical. Do not restate the subject or describe the
editing process.

Examples:

```text
feat(mru): record focused workspace paths

Store canonical paths instead of workspace IDs because IDs change between
Herdr sessions. Cap the list at 200 entries and replace it atomically.
```

```text
fix(zoxide): keep open workspaces when queries fail

Treat zoxide as an optional candidate source. Report the failure in the
popup without removing candidates returned by Herdr.
```

For a breaking change, add `!` before the colon and a `BREAKING CHANGE:` footer:

```text
feat!: require Herdr 0.9.0

BREAKING CHANGE: Older Herdr versions do not provide the snapshot fields
used to resolve workspace directories.
```

Each commit must build on its parent and contain one coherent change. Fold follow-up typo, format,
or review fixes into the commit that introduced them before review. Keep independent behavior in
independent commits.

Release commits use this exact subject:

```text
chore: release <version>
```

## Pull requests

A pull request should explain the observable result, the reason for the change, and the commands or
manual scenario used to verify it. Keep unrelated cleanup out of the diff. Call out platform-specific
behavior, state-file changes, external command changes, and release or checksum changes explicitly.

The pull request is ready for review when CI passes, user-visible changes have changelog entries,
and the commit series follows the convention above.
