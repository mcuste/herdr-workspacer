#!/usr/bin/env bash
# Starts an isolated Herdr session with demo workspaces and a demo zoxide database.
set -euo pipefail

project_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
# Keep application configuration in the user's standard config directory.
export XDG_CONFIG_HOME="$HOME/.config"
# Drop the variables of an outer Herdr pane so the demo never talks to that session.
unset HERDR_SESSION HERDR_SOCKET_PATH HERDR_CLIENT_SOCKET_PATH HERDR_BIN_PATH \
  HERDR_CONFIG_PATH HERDR_ENV HERDR_WORKSPACE_ID HERDR_TAB_ID HERDR_PANE_ID \
  HERDR_ACTIVE_WORKSPACE_ID HERDR_ACTIVE_TAB_ID HERDR_ACTIVE_PANE_ID HERDR_ACTIVE_PANE_CWD

session=${HERDR_WORKSPACER_DEMO_SESSION:-workspacer-demo}
demo_home=${HERDR_WORKSPACER_DEMO_HOME:-/tmp/hw}

for command in herdr zoxide cargo; do
  command -v "$command" >/dev/null || {
    printf 'Missing required command: %s\n' "$command" >&2
    exit 1
  }
done

installed_plugins=$(herdr plugin list)
case $installed_plugins in
*" herdr-workspacer "*) ;;
*)
  printf 'Install or link the plugin first: herdr plugin link %s\n' "$project_root" >&2
  exit 1
  ;;
esac

workspace_directories=(
  herdr-ws
  pi-diagram
  notes
)

# The second field is the visit count. Higher counts rank higher in the picker.
zoxide_directories=(
  "api-gateway 40"
  "web-client 32"
  "infra-terraform 24"
  "design-system 16"
  "blog 12"
  "quarterly-report 8"
  "onboarding 4"
)

cargo build --release --manifest-path "$project_root/Cargo.toml"
mkdir -p "$project_root/bin"
cp "$project_root/target/release/herdr-workspacer" "$project_root/bin/herdr-workspacer"

rm -rf "$demo_home"
mkdir -p "$demo_home"
export CLOUDSDK_CONFIG="$demo_home/gcloud"
unset CLOUDSDK_ACTIVE_CONFIG_NAME CLOUDSDK_CORE_ACCOUNT CLOUDSDK_CORE_PROJECT
export _ZO_DATA_DIR="$demo_home/zoxide"
mkdir -p "$_ZO_DATA_DIR"
mkdir -p "$CLOUDSDK_CONFIG"

for directory in "${workspace_directories[@]}"; do
  mkdir -p "$demo_home/$directory"
done

for record in "${zoxide_directories[@]}"; do
  read -r directory visits <<<"$record"
  mkdir -p "$demo_home/$directory"
  for _ in $(seq "$visits"); do
    zoxide add "$demo_home/$directory"
  done
done

herdr session stop "$session" >/dev/null 2>&1 || true
herdr session delete "$session" >/dev/null 2>&1 || true

session_socket() {
  herdr session list |
    awk -v name="$session" '$1 == name && $2 == "running" { print $NF }'
}

open_demo_workspaces() {
  local socket=""
  for _ in $(seq 100); do
    socket=$(session_socket)
    [[ -S $socket ]] && break
    sleep 0.2
  done
  [[ -S $socket ]] || return 0

  export HERDR_SOCKET_PATH="$socket"
  for directory in "${workspace_directories[@]:1}"; do
    herdr workspace create --cwd "$demo_home/$directory" \
      --label "${directory##*/}" --no-focus >/dev/null 2>&1 || true
  done
}

open_demo_workspaces &

cd "$demo_home/${workspace_directories[0]}"
clear
exec herdr --session "$session"
