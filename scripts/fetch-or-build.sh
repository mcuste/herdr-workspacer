#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

bin_dir="$root/bin"
binary="$bin_dir/herdr-workspacer"
version=$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml)

if [ -z "$version" ]; then
    printf '%s\n' 'Could not read the plugin version from Cargo.toml.' >&2
    exit 1
fi

case $(uname -s) in
    Darwin) platform=macos ;;
    Linux) platform=linux ;;
    *)
        printf 'Unsupported operating system: %s\n' "$(uname -s)" >&2
        exit 1
        ;;
esac

case $(uname -m) in
    x86_64) architecture=x86_64 ;;
    arm64 | aarch64) architecture=aarch64 ;;
    *)
        printf 'Unsupported architecture: %s\n' "$(uname -m)" >&2
        exit 1
        ;;
esac

asset="herdr-workspacer-${platform}-${architecture}"
temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/herdr-workspacer.XXXXXX")
cleanup() {
    rm -rf "$temporary_dir"
}
trap cleanup EXIT HUP INT TERM

if command -v curl >/dev/null 2>&1 \
    && curl --fail --location --silent --show-error \
        --output "$temporary_dir/$asset" \
        "https://github.com/mcuste/herdr-workspacer/releases/download/v${version}/$asset" \
    && curl --fail --location --silent --show-error \
        --output "$temporary_dir/SHA256SUMS" \
        "https://github.com/mcuste/herdr-workspacer/releases/download/v${version}/SHA256SUMS"; then
    expected=$(awk -v asset="$asset" '$2 == asset { print $1; exit }' "$temporary_dir/SHA256SUMS")
    if command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$temporary_dir/$asset" | awk '{ print $1 }')
    elif command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$temporary_dir/$asset" | awk '{ print $1 }')
    else
        printf '%s\n' 'No SHA-256 tool is available to verify the downloaded binary.' >&2
        exit 1
    fi

    if [ -z "$expected" ] || [ "$actual" != "$expected" ]; then
        printf '%s\n' 'Downloaded binary checksum does not match SHA256SUMS.' >&2
        exit 1
    fi

    mkdir -p "$bin_dir"
    install -m 755 "$temporary_dir/$asset" "$binary"
    exit 0
fi

if command -v cargo >/dev/null 2>&1; then
    cargo build --release --locked
    mkdir -p "$bin_dir"
    install -m 755 target/release/herdr-workspacer "$binary"
    exit 0
fi

printf '%s\n' "No release binary is available for ${platform}/${architecture}, and Cargo is not installed." >&2
exit 1
