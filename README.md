# Herdr Workspacer

MRU fuzzy workspace picker for Herdr, using open workspaces and optional zoxide directories.

## Features

- Opens as a centered Herdr popup.
- Lists open workspaces and zoxide directories.
- Keeps open workspaces when zoxide is unavailable.
- Suppresses zoxide entries that duplicate open workspace paths.
- Stores recency by canonical directory path.
- Uses fuzzy matching to filter without changing MRU order.
- Focuses an existing workspace or creates one for the selected directory.

## Requirements

- Herdr 0.8.0 or later on macOS or Linux.
- zoxide is optional. It supplies additional directory candidates.

The plugin does not require `fzf`, `jq`, Rust, or Cargo when a release binary is available.

## Install

```sh
herdr plugin install mcuste/herdr-workspacer
```

Choose any Herdr keybinding. For example, add this to `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+s"
type = "plugin_action"
command = "herdr-workspacer.open"
description = "open workspace picker"
```

Reload the configuration:

```sh
herdr server reload-config
```

Herdr lists active bindings with `prefix+?`. The action also works without a keybinding:

```sh
herdr plugin action invoke herdr-workspacer.open
```

## Controls

| Key | Action |
| --- | --- |
| Text | Filter candidates |
| Backspace | Remove one query character |
| Up or `ctrl+p` | Move to the previous result |
| Down or `ctrl+n` | Move to the next result |
| PageUp or PageDown | Move ten results |
| `ctrl+u` | Clear the query |
| Enter | Focus or create the selected workspace |
| Esc or `ctrl+c` | Close the picker |

MRU order remains unchanged while a query filters the visible results.

## zoxide fallback

If zoxide is not installed or its query fails, the picker still lists open Herdr workspaces and displays a warning. Stale and malformed zoxide entries are skipped.

## Uninstall

```sh
herdr plugin uninstall herdr-workspacer
```

## Development

Requires Rust stable, Cargo, Herdr 0.8.0 or later, and zoxide for full-source testing.

```sh
cargo build
mkdir -p bin
cp target/debug/herdr-workspacer bin/herdr-workspacer
herdr plugin link .
herdr plugin action invoke herdr-workspacer.open
```

Run the project checks with:

```sh
just verify
```

## License

MIT. See [LICENSE](LICENSE).
