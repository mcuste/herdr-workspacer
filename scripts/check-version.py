#!/usr/bin/env python3
"""Validate release metadata versions."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def read_version(path: Path, key: str) -> str:
    with path.open("rb") as source:
        data = tomllib.load(source)
    if key == "package":
        return data["package"]["version"]
    return data["version"]


def main() -> None:
    cargo_version = read_version(ROOT / "Cargo.toml", "package")
    plugin_version = read_version(ROOT / "herdr-plugin.toml", "plugin")

    if cargo_version != plugin_version:
        raise SystemExit(
            "Cargo.toml and herdr-plugin.toml versions must match: "
            f"{cargo_version!r} != {plugin_version!r}"
        )

    if len(sys.argv) == 2 and sys.argv[1] != f"v{cargo_version}":
        raise SystemExit(
            f"release tag must be v{cargo_version!s}, got {sys.argv[1]!r}"
        )


if __name__ == "__main__":
    main()
